import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import {
  expectedWheelFilenames,
  normalizeDistributionName,
  normalizePep440Version,
  parsePyprojectName,
} from "./check-pypi-published.mjs";
import {
  cleanupTempRepo,
  createTempRepo,
} from "./test-harness.mjs";

const SCRIPT_PATH = fileURLToPath(
  new URL("./check-pypi-published.mjs", import.meta.url),
);

// Fake PyPI JSON API. Responses are keyed by the SemVer in the request path so
// every case is self-contained and tests never depend on the live registry.
// The server lives in THIS process, so the CLI child must be spawned
// asynchronously (spawnSync would block this event loop and deadlock the
// probe against the server).
let server;
let baseUrl;

// Per-version probe counts so a case can flip behavior mid-retry (transient 404).
const probeCounts = new Map();

const wheelUrls = (version) =>
  expectedWheelFilenames("spoke-connect", version).map((filename) => ({
    filename,
  }));

before(async () => {
  server = createServer((req, res) => {
    const match = req.url?.match(/^\/pypi\/spoke-connect\/([^/]+)\/json$/);
    const version = match?.[1] ?? "";
    const json = (status, body) => {
      res.writeHead(status, { "content-type": "application/json" });
      res.end(JSON.stringify(body));
    };
    switch (version) {
      case "0.1.0": // absent — not published at this SemVer
        json(404, { message: "not found" });
        break;
      case "0.2.0": // full expected set present
        json(200, { urls: wheelUrls("0.2.0") });
        break;
      case "0.3.0": // partial — win_amd64 wheel missing
        json(200, {
          urls: wheelUrls("0.3.0").filter((u) => !u.filename.includes("win_amd64")),
        });
        break;
      case "0.4.0": // registry error
        res.writeHead(503, { "content-type": "text/plain" });
        res.end("temporarily unavailable");
        break;
      case "0.5.0": // malformed JSON
        res.writeHead(200, { "content-type": "application/json" });
        res.end("{ this is not json");
        break;
      case "0.6.0": // 200 but unexpected payload shape
        json(200, { info: {} });
        break;
      case "0.7.0": // all expected wheels listed, but one yanked (PEP 592)
        json(200, {
          urls: wheelUrls("0.7.0").map((u) =>
            u.filename.includes("macosx") ? { ...u, yanked: true } : u,
          ),
        });
        break;
      case "0.8.0-alpha.1": // prerelease tag: full set with PEP 440-normalized filenames
        json(200, { urls: wheelUrls("0.8.0-alpha.1") });
        break;
      case "0.12.0": // full set plus an unexpected entry — warning only
        json(200, {
          urls: [...wheelUrls("0.12.0"), { filename: "spoke_connect-0.12.0.tar.gz" }],
        });
        break;
      case "0.13.0": // transient 404: absent for the first two probes, then full set
        probeCounts.set(version, (probeCounts.get(version) ?? 0) + 1);
        if (probeCounts.get(version) <= 2) {
          json(404, { message: "not found" });
        } else {
          json(200, { urls: wheelUrls("0.13.0") });
        }
        break;
      default:
        json(404, { message: "not found" });
        break;
    }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  baseUrl = `http://127.0.0.1:${address.port}`;
});

after(async () => {
  // fetch (undici) keeps the probe connection alive; closeAllConnections()
  // unblocks server.close() so the test process can exit.
  server.closeAllConnections();
  await new Promise((resolve, reject) =>
    server.close((err) => (err ? reject(err) : resolve())),
  );
});

/**
 * Rewrite the `version = "…"` line in the `[project]` table of a temp fixture
 * pyproject.toml so the lockstep cross-check matches the RELEASE_TAG under test.
 *
 * @param {string} repoRoot
 * @param {string} version
 */
function setFixturePyprojectVersion(repoRoot, version) {
  const pyprojectPath = join(
    repoRoot,
    "crates/spoke-connect/bindings/python/pyproject.toml",
  );
  const contents = readFileSync(pyprojectPath, "utf8");
  const updated = contents.replace(
    /^version\s*=\s*"[^"]+"/m,
    `version = "${version}"`,
  );
  assert.notEqual(updated, contents, "fixture pyproject version line not found");
  writeFileSync(pyprojectPath, updated);
}

/**
 * Spawn the CLI against the fake PyPI API from a temp lockstep fixture repo.
 *
 * @param {object} opts
 * @param {string} opts.tag RELEASE_TAG value (e.g. "v0.2.0").
 * @param {string[]} [opts.args] Extra CLI args (e.g. ["--verify"]).
 * @param {string} [opts.githubOutput] When set, point GITHUB_OUTPUT at this file.
 * @param {Record<string, string>} [opts.extraEnv] Extra env vars for the child
 *   (e.g. PYPI_VERIFY_RETRY_BASE_MS to keep retry sleeps short).
 * @returns {Promise<{ status: number | null; stdout: string; stderr: string }>}
 */
async function runCheck({ tag, args = [], githubOutput, extraEnv = {} }) {
  const repoRoot = createTempRepo();
  setFixturePyprojectVersion(repoRoot, tag.slice(1));
  const env = {
    ...process.env,
    SPOKE_REPO_ROOT: repoRoot,
    RELEASE_TAG: tag,
    PYPI_BASE_URL: baseUrl,
    ...extraEnv,
  };
  delete env.GITHUB_OUTPUT;
  if (githubOutput) {
    env.GITHUB_OUTPUT = githubOutput;
  }
  try {
    return await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [SCRIPT_PATH, ...args], {
        cwd: repoRoot,
        env,
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (chunk) => {
        stdout += chunk;
      });
      child.stderr.on("data", (chunk) => {
        stderr += chunk;
      });
      child.on("error", reject);
      child.on("close", (status) => resolve({ status, stdout, stderr }));
    });
  } finally {
    cleanupTempRepo(repoRoot);
  }
}

describe("check-pypi-published.mjs CLI", () => {
  it("reports publish needed when the version is absent (404)", async () => {
    const result = await runCheck({ tag: "v0.1.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /not found on PyPI \(HTTP 404\)/);
    assert.match(result.stdout, /publish_needed=true/);
    assert.match(
      result.stdout,
      /missing_files=spoke_connect-0\.1\.0-py3-none-manylinux_2_35_x86_64\.whl,spoke_connect-0\.1\.0-py3-none-macosx_11_0_arm64\.whl,spoke_connect-0\.1\.0-py3-none-win_amd64\.whl/,
    );
  });

  it("skips when the full expected set is present", async () => {
    const result = await runCheck({ tag: "v0.2.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /already published on PyPI \(3\/3 expected files present\)/);
    assert.match(result.stdout, /publish_needed=false/);
    assert.match(result.stdout, /missing_files=$/m);
  });

  it("does not skip-green on a partial set", async () => {
    const result = await runCheck({ tag: "v0.3.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /partially published on PyPI/);
    assert.match(result.stdout, /publish_needed=true/);
    assert.match(
      result.stdout,
      /missing_files=spoke_connect-0\.3\.0-py3-none-win_amd64\.whl/,
    );
    assert.doesNotMatch(result.stdout, /publish_needed=false/);
  });

  it("writes publish_needed and missing_files to GITHUB_OUTPUT", async () => {
    const outDir = mkdtempSync(join(tmpdir(), "pypi-gh-output-"));
    const outFile = join(outDir, "step-output");
    try {
      const result = await runCheck({ tag: "v0.2.0", githubOutput: outFile });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const written = readFileSync(outFile, "utf8");
      assert.match(written, /publish_needed=false/);
      assert.match(written, /missing_files=$/m);
    } finally {
      rmSync(outDir, { recursive: true, force: true });
    }
  });

  it("exits non-zero on registry error (503)", async () => {
    const result = await runCheck({ tag: "v0.4.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /HTTP 503/);
  });

  it("exits non-zero on malformed JSON", async () => {
    const result = await runCheck({ tag: "v0.5.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /malformed JSON/);
  });

  it("exits non-zero when the payload is missing the urls array", async () => {
    const result = await runCheck({ tag: "v0.6.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /"urls"/);
  });

  it("requires a v-prefixed RELEASE_TAG", async () => {
    const result = await runCheck({ tag: "0.2.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /RELEASE_TAG/);
  });

  it("--verify passes when the full set is present", async () => {
    const result = await runCheck({ tag: "v0.2.0", args: ["--verify"] });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /confirmed full expected set/);
  });

  it("--verify fails on a partial set", async () => {
    const result = await runCheck({ tag: "v0.3.0", args: ["--verify"] });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /missing/);
  });

  it("--verify fails when the version is absent", async () => {
    const result = await runCheck({ tag: "v0.1.0", args: ["--verify"] });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /missing/);
  });

  it("--verify retries a transient 404 and succeeds within the deadline", async () => {
    const result = await runCheck({
      tag: "v0.13.0",
      args: ["--verify", "--verify-retry-seconds", "1"],
      extraEnv: { PYPI_VERIFY_RETRY_BASE_MS: "10" },
    });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /confirmed full expected set/);
    assert.ok(probeCounts.get("0.13.0") >= 3, "expected at least 3 probes");
  });

  it("--verify retries then fails loud when the deadline expires with files missing", async () => {
    const result = await runCheck({
      tag: "v0.3.0",
      args: ["--verify", "--verify-retry-seconds", "0.2"],
      extraEnv: { PYPI_VERIFY_RETRY_BASE_MS: "10" },
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /missing/);
    assert.match(result.stderr, /win_amd64/);
  });

  it("--verify-retry-seconds 0 keeps single-shot semantics", async () => {
    const result = await runCheck({
      tag: "v0.1.0",
      args: ["--verify", "--verify-retry-seconds", "0"],
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /missing/);
  });

  it("treats a yanked wheel as absent (pre-check → publish needed)", async () => {
    const result = await runCheck({ tag: "v0.7.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /partially published on PyPI/);
    assert.match(result.stdout, /publish_needed=true/);
    assert.match(
      result.stdout,
      /missing_files=spoke_connect-0\.7\.0-py3-none-macosx_11_0_arm64\.whl/,
    );
    assert.doesNotMatch(result.stdout, /publish_needed=false/);
  });

  it("--verify reds while any expected wheel is yanked", async () => {
    const result = await runCheck({ tag: "v0.7.0", args: ["--verify"] });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /missing/);
    assert.match(result.stderr, /macosx_11_0_arm64/);
  });

  it("skips a prerelease tag whose PEP 440-normalized wheel filenames are present", async () => {
    const result = await runCheck({ tag: "v0.8.0-alpha.1" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /already published on PyPI \(3\/3 expected files present\)/);
    assert.match(result.stdout, /publish_needed=false/);
    assert.match(result.stdout, /missing_files=$/m);
  });

  it("--verify passes for a prerelease tag with normalized filenames", async () => {
    const result = await runCheck({ tag: "v0.8.0-alpha.1", args: ["--verify"] });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /confirmed full expected set/);
  });

  it("warns on unexpected registry entries without changing the verdict", async () => {
    const result = await runCheck({ tag: "v0.12.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stderr, /warning/);
    assert.match(result.stderr, /spoke_connect-0\.12\.0\.tar\.gz/);
    assert.match(result.stdout, /publish_needed=false/);
  });

  it("exits non-zero when the tag version mismatches pyproject.toml", async () => {
    // Fixture pyproject version is rewritten to the tag by runCheck, so
    // simulate drift by rewriting it to a different version afterwards.
    const repoRoot = createTempRepo();
    setFixturePyprojectVersion(repoRoot, "0.2.0");
    setFixturePyprojectVersion(repoRoot, "9.9.9");
    const env = {
      ...process.env,
      SPOKE_REPO_ROOT: repoRoot,
      RELEASE_TAG: "v0.2.0",
      PYPI_BASE_URL: baseUrl,
    };
    delete env.GITHUB_OUTPUT;
    try {
      const result = await new Promise((resolve, reject) => {
        const child = spawn(process.execPath, [SCRIPT_PATH], {
          cwd: repoRoot,
          env,
          stdio: ["ignore", "pipe", "pipe"],
        });
        let stdout = "";
        let stderr = "";
        child.stdout.on("data", (chunk) => {
          stdout += chunk;
        });
        child.stderr.on("data", (chunk) => {
          stderr += chunk;
        });
        child.on("error", reject);
        child.on("close", (status) => resolve({ status, stdout, stderr }));
      });
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /pyproject\.toml/);
      assert.match(result.stderr, /9\.9\.9/);
      assert.match(result.stderr, /0\.2\.0/);
    } finally {
      cleanupTempRepo(repoRoot);
    }
  });
});

describe("check-pypi-published.mjs expected set", () => {
  it("derives exactly the three locked platform wheels (no sdist)", () => {
    assert.deepEqual(expectedWheelFilenames("spoke-connect", "0.10.0"), [
      "spoke_connect-0.10.0-py3-none-manylinux_2_35_x86_64.whl",
      "spoke_connect-0.10.0-py3-none-macosx_11_0_arm64.whl",
      "spoke_connect-0.10.0-py3-none-win_amd64.whl",
    ]);
  });

  it("normalizes the distribution name per PEP 503", () => {
    assert.equal(normalizeDistributionName("spoke-connect"), "spoke_connect");
  });

  it("parses the [project] name from pyproject.toml", () => {
    assert.equal(
      parsePyprojectName('[project]\nname = "spoke-connect"\nversion = "0.10.0"\n'),
      "spoke-connect",
    );
    assert.equal(parsePyprojectName('[tool.foo]\nname = "other"\n'), null);
  });

  it("normalizes prerelease versions per PEP 440", () => {
    assert.equal(normalizePep440Version("0.2.0-alpha.1"), "0.2.0a1");
    assert.equal(normalizePep440Version("0.2.0-beta.1"), "0.2.0b1");
    assert.equal(normalizePep440Version("0.2.0-rc.1"), "0.2.0rc1");
    assert.equal(normalizePep440Version("0.2.0-rc1"), "0.2.0rc1");
    assert.equal(normalizePep440Version("0.2.0-dev.3"), "0.2.0.dev3");
    assert.equal(normalizePep440Version("0.10.0"), "0.10.0");
  });

  it("builds PEP 440-normalized wheel filenames for prerelease versions", () => {
    assert.deepEqual(expectedWheelFilenames("spoke-connect", "0.2.0-alpha.1"), [
      "spoke_connect-0.2.0a1-py3-none-manylinux_2_35_x86_64.whl",
      "spoke_connect-0.2.0a1-py3-none-macosx_11_0_arm64.whl",
      "spoke_connect-0.2.0a1-py3-none-win_amd64.whl",
    ]);
  });
});
