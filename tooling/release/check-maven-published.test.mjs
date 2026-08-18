import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import {
  expectedArtifactNames,
  listZipEntryNames,
} from "./check-maven-published.mjs";
import {
  cleanupTempRepo,
  createTempRepo,
} from "./test-harness.mjs";

const SCRIPT_PATH = fileURLToPath(
  new URL("./check-maven-published.mjs", import.meta.url),
);

// Fixture jar (built with `zip`, embedded as base64 so tests need no external
// tools at runtime):
//  - JNA_JAR_FULL_B64 carries all three locked JNA resource entries plus an
//    unrelated class entry (extra entries must be ignored).
//  - JNA_JAR_PARTIAL_B64 carries only linux-x86-64 + darwin-aarch64 (the
//    win32-x86-64 entry is missing — "jar missing natives" partial case).
const JNA_JAR_FULL_B64 =
  "UEsDBAoAAAAAAKh+EV0AAAAAAAAAAAAAAAAgAAAAbGludXgteDg2LTY0L2xpYnNwb2tlX2Nvbm5l" +
  "Y3Quc29QSwMECgAAAAAAqH4RXQAAAAAAAAAAAAAAACUAAABkYXJ3aW4tYWFyY2g2NC9saWJzcG9r" +
  "ZV9jb25uZWN0LmR5bGliUEsDBAoAAAAAAKh+EV0AAAAAAAAAAAAAAAAeAAAAd2luMzIteDg2LTY0" +
  "L3Nwb2tlX2Nvbm5lY3QuZGxsUEsDBAoAAAAAAKh+EV0AAAAAAAAAAAAAAAAgAAAAdW5pZmZpL3Nw" +
  "b2tlX2Nvbm5lY3QvRHVtbXkuY2xhc3NQSwECHgMKAAAAAACofhFdAAAAAAAAAAAAAAAAIAAAAAAA" +
  "AAAAAAAApIEAAAAAbGludXgteDg2LTY0L2xpYnNwb2tlX2Nvbm5lY3Quc29QSwECHgMKAAAAAACo" +
  "fhFdAAAAAAAAAAAAAAAAJQAAAAAAAAAAAAAApIE+AAAAZGFyd2luLWFhcmNoNjQvbGlic3Bva2Vf" +
  "Y29ubmVjdC5keWxpYlBLAQIeAwoAAAAAAKh+EV0AAAAAAAAAAAAAAAAeAAAAAAAAAAAAAACkgYEA" +
  "AAB3aW4zMi14ODYtNjQvc3Bva2VfY29ubmVjdC5kbGxQSwECHgMKAAAAAACofhFdAAAAAAAAAAAA" +
  "AAAAIAAAAAAAAAAAAAAApIG9AAAAdW5pZmZpL3Nwb2tlX2Nvbm5lY3QvRHVtbXkuY2xhc3NQSwUG" +
  "AAAAAAQABAA7AQAA+wAAAAAA";
const JNA_JAR_PARTIAL_B64 =
  "UEsDBAoAAAAAAKh+EV0AAAAAAAAAAAAAAAAgAAAAbGludXgteDg2LTY0L2xpYnNwb2tlX2Nvbm5l" +
  "Y3Quc29QSwMECgAAAAAAqH4RXQAAAAAAAAAAAAAAACUAAABkYXJ3aW4tYWFyY2g2NC9saWJzcG9r" +
  "ZV9jb25uZWN0LmR5bGliUEsBAh4DCgAAAAAAqH4RXQAAAAAAAAAAAAAAACAAAAAAAAAAAAAAAKSB" +
  "AAAAAGxpbnV4LXg4Ni02NC9saWJzcG9rZV9jb25uZWN0LnNvUEsBAh4DCgAAAAAAqH4RXQAAAAAA" +
  "AAAAAAAAACUAAAAAAAAAAAAAAKSBPgAAAGRhcndpbi1hYXJjaDY0L2xpYnNwb2tlX2Nvbm5lY3Qu" +
  "ZHlsaWJQSwUGAAAAAAIAAgChAAAAgQAAAAAA";

// Fake GitHub Packages Maven registry. Responses are keyed by the SemVer in
// the request path so every case is self-contained and tests never depend on
// the live registry. The server lives in THIS process, so the CLI child must
// be spawned asynchronously (spawnSync would block this event loop and
// deadlock the probe against the server).
let server;
let baseUrl;

before(async () => {
  server = createServer((req, res) => {
    const match = req.url?.match(
      /^\/dev\/42ch\/spoke-connect\/([^/]+)\/(spoke-connect-[^/]+\.(pom|module|jar))$/,
    );
    const version = match?.[1] ?? "";
    const filename = match?.[2] ?? "";
    const ext = match?.[3] ?? "";
    const serve = (status, body, contentType = "text/plain") => {
      res.writeHead(status, { "content-type": contentType });
      res.end(body);
    };
    const jarFor = (b64) => Buffer.from(b64, "base64");
    switch (version) {
      case "0.1.0": // absent — nothing published at this SemVer
        serve(404, "not found");
        break;
      case "0.2.0": // full expected set present (pom + module + jar with all JNA entries)
        if (ext === "jar") {
          serve(200, jarFor(JNA_JAR_FULL_B64), "application/java-archive");
        } else {
          serve(200, ext === "pom" ? '<?xml version="1.0" encoding="UTF-8"?>\n<project/>' : '{"formatVersion":"1.1"}', "application/xml");
        }
        break;
      case "0.3.0": // partial — pom only (module + jar missing)
        if (ext === "pom") {
          serve(200, '<?xml version="1.0" encoding="UTF-8"?>\n<project/>', "application/xml");
        } else {
          serve(404, "not found");
        }
        break;
      case "0.4.0": // partial — jar present but missing the win32-x86-64 JNA entry
        if (ext === "jar") {
          serve(200, jarFor(JNA_JAR_PARTIAL_B64), "application/java-archive");
        } else {
          serve(200, ext === "pom" ? '<?xml version="1.0" encoding="UTF-8"?>\n<project/>' : '{"formatVersion":"1.1"}', "application/xml");
        }
        break;
      case "0.5.0": // auth failure — 401
        serve(401, "Bad credentials");
        break;
      case "0.6.0": // auth failure — 403
        serve(403, "Forbidden");
        break;
      case "0.7.0": // registry error
        serve(503, "temporarily unavailable");
        break;
      case "0.8.0": // jar 200 but not a ZIP archive
        if (ext === "jar") {
          serve(200, "this is definitely not a zip archive", "application/java-archive");
        } else {
          serve(200, ext === "pom" ? '<?xml version="1.0" encoding="UTF-8"?>\n<project/>' : '{"formatVersion":"1.1"}', "application/xml");
        }
        break;
      case "0.9.0": // pom 200 but an HTML error page — must fail loud
        if (ext === "pom") {
          serve(200, "<html><body>not a pom</body></html>", "text/html");
        } else {
          serve(404, "not found");
        }
        break;
      case "0.12.0": // module 200 but an HTML error page — must fail loud
        if (ext === "module") {
          serve(200, "<html><body>not a module</body></html>", "text/html");
        } else {
          serve(200, '<?xml version="1.0" encoding="UTF-8"?>\n<project/>', "application/xml");
        }
        break;
      case "0.11.0": // full set; module metadata lists an extra file — warning only
        if (ext === "jar") {
          serve(200, jarFor(JNA_JAR_FULL_B64), "application/java-archive");
        } else if (ext === "pom") {
          serve(200, '<?xml version="1.0" encoding="UTF-8"?>\n<project/>', "application/xml");
        } else {
          serve(
            200,
            JSON.stringify({
              formatVersion: "1.1",
              files: [
                { name: "spoke-connect-0.11.0.pom", url: "spoke-connect-0.11.0.pom" },
                { name: "spoke-connect-0.11.0.module", url: "spoke-connect-0.11.0.module" },
                { name: "spoke-connect-0.11.0.jar", url: "spoke-connect-0.11.0.jar" },
                { name: "spoke-connect-0.11.0-sources.jar", url: "spoke-connect-0.11.0-sources.jar" },
              ],
            }),
            "application/json",
          );
        }
        break;
      default:
        serve(404, "not found");
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
 * Rewrite the `version = "…"` line in a temp fixture build.gradle.kts so the
 * lockstep cross-check matches the RELEASE_TAG under test. Tolerates a no-op
 * when the fixture already sits at the target version (line is asserted, not
 * the replace result).
 *
 * @param {string} repoRoot
 * @param {string} version
 */
function setFixtureGradleVersion(repoRoot, version) {
  const gradlePath = join(
    repoRoot,
    "crates/spoke-connect/bindings/kotlin/build.gradle.kts",
  );
  const contents = readFileSync(gradlePath, "utf8");
  // Assert the version LINE exists (not that the replace changed something):
  // when the repo already sits at the target version the rewrite is a no-op.
  const versionLine = /^version\s*=\s*"[^"]+"/m;
  assert.match(contents, versionLine, "fixture gradle version line not found");
  const updated = contents.replace(versionLine, `version = "${version}"`);
  if (updated !== contents) {
    writeFileSync(gradlePath, updated);
  }
}

/**
 * Spawn the CLI against the fake registry from a temp lockstep fixture repo.
 *
 * @param {object} opts
 * @param {string} opts.tag RELEASE_TAG value (e.g. "v0.2.0").
 * @param {string[]} [opts.args] Extra CLI args (e.g. ["--verify"]).
 * @param {string} [opts.githubOutput] When set, point GITHUB_OUTPUT at this file.
 * @param {boolean} [opts.omitToken] When true, drop GITHUB_TOKEN from the env.
 * @returns {Promise<{ status: number | null; stdout: string; stderr: string }>}
 */
async function runCheck({ tag, args = [], githubOutput, omitToken = false }) {
  const repoRoot = createTempRepo();
  setFixtureGradleVersion(repoRoot, tag.slice(1));
  const env = {
    ...process.env,
    SPOKE_REPO_ROOT: repoRoot,
    RELEASE_TAG: tag,
    MAVEN_BASE_URL: baseUrl,
    GITHUB_TOKEN: "test-token",
  };
  delete env.GITHUB_OUTPUT;
  if (omitToken) {
    delete env.GITHUB_TOKEN;
  }
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

describe("check-maven-published.mjs CLI", () => {
  it("reports publish needed when the version is absent (404)", async () => {
    const result = await runCheck({ tag: "v0.1.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /not found on GitHub Packages Maven \(HTTP 404\)/);
    assert.match(result.stdout, /publish_needed=true/);
    assert.match(
      result.stdout,
      /missing_files=spoke-connect-0\.1\.0\.pom,spoke-connect-0\.1\.0\.module,spoke-connect-0\.1\.0\.jar/,
    );
  });

  it("skips when the full expected set is present (pom + module + jar with JNA natives)", async () => {
    const result = await runCheck({ tag: "v0.2.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(
      result.stdout,
      /already published on GitHub Packages Maven \(3\/3 expected files present/,
    );
    assert.match(result.stdout, /publish_needed=false/);
    assert.match(result.stdout, /missing_files=$/m);
  });

  it("does not skip-green on a partial set (pom only)", async () => {
    const result = await runCheck({ tag: "v0.3.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /partially published on GitHub Packages Maven/);
    assert.match(result.stdout, /publish_needed=true/);
    assert.match(
      result.stdout,
      /missing_files=spoke-connect-0\.3\.0\.module,spoke-connect-0\.3\.0\.jar/,
    );
    assert.doesNotMatch(result.stdout, /publish_needed=false/);
  });

  it("does not skip-green when the jar is missing JNA natives", async () => {
    const result = await runCheck({ tag: "v0.4.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /partially published on GitHub Packages Maven/);
    assert.match(result.stdout, /publish_needed=true/);
    assert.match(
      result.stdout,
      /missing_files=win32-x86-64\/spoke_connect\.dll/,
    );
    assert.doesNotMatch(result.stdout, /publish_needed=false/);
  });

  it("writes publish_needed and missing_files to GITHUB_OUTPUT", async () => {
    const outDir = mkdtempSync(join(tmpdir(), "maven-gh-output-"));
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

  it("exits non-zero on auth failure (401)", async () => {
    const result = await runCheck({ tag: "v0.5.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /401/);
    assert.match(result.stderr, /authentication/i);
  });

  it("exits non-zero on auth failure (403)", async () => {
    const result = await runCheck({ tag: "v0.6.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /403/);
    assert.match(result.stderr, /authentication/i);
  });

  it("exits non-zero on registry error (503)", async () => {
    const result = await runCheck({ tag: "v0.7.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /HTTP 503/);
  });

  it("exits non-zero when the jar payload is not a ZIP archive", async () => {
    const result = await runCheck({ tag: "v0.8.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /ZIP/i);
  });

  it("exits non-zero when GITHUB_TOKEN is missing", async () => {
    const result = await runCheck({ tag: "v0.2.0", omitToken: true });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /GITHUB_TOKEN/);
  });

  it("exits non-zero when the tag version mismatches build.gradle.kts", async () => {
    // Fixture gradle version is rewritten to the tag by runCheck, so simulate
    // drift by rewriting it to a different version afterwards.
    const repoRoot = createTempRepo();
    setFixtureGradleVersion(repoRoot, "0.2.0");
    setFixtureGradleVersion(repoRoot, "9.9.9");
    const env = {
      ...process.env,
      SPOKE_REPO_ROOT: repoRoot,
      RELEASE_TAG: "v0.2.0",
      MAVEN_BASE_URL: baseUrl,
      GITHUB_TOKEN: "test-token",
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
      assert.match(result.stderr, /build\.gradle\.kts/);
      assert.match(result.stderr, /0\.2\.0/);
    } finally {
      cleanupTempRepo(repoRoot);
    }
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

  it("--verify fails when the jar is missing JNA natives", async () => {
    const result = await runCheck({ tag: "v0.4.0", args: ["--verify"] });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /missing/);
  });

  it("fails loud when a .pom response is not XML (HTML error page)", async () => {
    const result = await runCheck({ tag: "v0.9.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /non-XML body/);
    assert.match(result.stderr, /\.pom/);
  });

  it("fails loud when a .module response is not JSON (HTML error page)", async () => {
    const result = await runCheck({ tag: "v0.12.0" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /non-JSON body/);
    assert.match(result.stderr, /\.module/);
  });

  it("warns on module-metadata files outside the expected set without changing the verdict", async () => {
    const result = await runCheck({ tag: "v0.11.0" });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stderr, /warning/);
    assert.match(result.stderr, /spoke-connect-0\.11\.0-sources\.jar/);
    assert.match(result.stdout, /publish_needed=false/);
  });
});

describe("check-maven-published.mjs expected set", () => {
  it("derives exactly the locked Maven artifact set (pom + module + jar, no classifiers)", () => {
    assert.deepEqual(expectedArtifactNames("0.10.0"), [
      "spoke-connect-0.10.0.pom",
      "spoke-connect-0.10.0.module",
      "spoke-connect-0.10.0.jar",
    ]);
  });

  it("lists JNA resource entries from a real ZIP buffer", () => {
    const names = listZipEntryNames(Buffer.from(JNA_JAR_FULL_B64, "base64"));
    assert.ok(names.includes("linux-x86-64/libspoke_connect.so"));
    assert.ok(names.includes("darwin-aarch64/libspoke_connect.dylib"));
    assert.ok(names.includes("win32-x86-64/spoke_connect.dll"));
    assert.ok(names.includes("uniffi/spoke_connect/Dummy.class"));
    assert.equal(names.length, 4);
  });

  it("rejects a non-ZIP buffer", () => {
    assert.throws(
      () => listZipEntryNames(Buffer.from("this is not a zip archive", "utf8")),
      /ZIP/,
    );
  });
});
