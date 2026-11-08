import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";

const required = [
  "Build Swift xcframework (staged, locked)",
  "Write xcframework hash manifest",
  "Record xcframework build provenance",
  "Upload xcframework artifact",
];
const freshness = "Verify committed xcframework freshness";
const smoke = "Fail on committed Swift consumer smoke";
const digest = (text) => createHash("sha256").update(text).digest("hex");
const bundle = "crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework";

function runFixture({
  failures = [freshness], missing, invalidSource, invalidManifest, invalidProvenance, invalidCheckout,
  status = "completed", conclusion = "failure",
  jobStatus = "completed", jobConclusion, cancelledStep, advanceHead, allowSha, transferWithoutDelivery,
} = {}) {
  const root = mkdtempSync(join(tmpdir(), "xcframework-apply-"));
  const put = (path, content, options) => {
    const absolute = join(root, path);
    mkdirSync(dirname(absolute), { recursive: true });
    writeFileSync(absolute, content, options);
  };
  const env = {
    ...process.env,
    GIT_AUTHOR_NAME: "Fixture", GIT_AUTHOR_EMAIL: "fixture@example.invalid",
    GIT_COMMITTER_NAME: "Fixture", GIT_COMMITTER_EMAIL: "fixture@example.invalid",
  };
  const git = (...args) => execFileSync("git", args, { cwd: root, env, encoding: "utf8" }).trim();
  try {
    const script = join(root, "tooling/connect/apply-xcframework-artifact.sh");
    mkdirSync(dirname(script), { recursive: true });
    copyFileSync(fileURLToPath(new URL("./apply-xcframework-artifact.sh", import.meta.url)), script);
    put(".gitattributes", `${bundle}/**/*.a filter=lfs diff=lfs merge=lfs -text\n`);
    put(".github/workflows/xcframework.yml", 'toolchain: 1.98.1\ntargets: aarch64-apple-darwin\nXCFRAMEWORK_LOCKED: "1"\n');
    put(`${bundle}/Info.plist`, "old plist\n");
    put(`${bundle}/macos-arm64/libspoke_connect.a`, "old native\n");
    git("init", "-q");
    git("add", ".");
    git("commit", "-qm", "fixture");
    const producerSha = git("rev-parse", "HEAD");
    if (advanceHead) git("commit", "--allow-empty", "-qm", "advance the checkout past the producer");
    const head = git("rev-parse", "HEAD");
    const artifact = "artifact/spoke_connectFFI.xcframework";
    const plist = "new plist\n";
    const native = "new native\n";
    const manifest = `${digest(plist)}  ./Info.plist\n${invalidManifest ? "0".repeat(64) : digest(native)}  ./macos-arm64/libspoke_connect.a\n`;
    put(`${artifact}/Info.plist`, plist);
    put(`${artifact}/macos-arm64/libspoke_connect.a`, native);
    // Every committed bundle file mirrors its artifact counterpart in exact
    // size and mtime with different bytes, so the default rsync size/mtime
    // shortcut skips them deterministically and would silently leave the old
    // bytes. Delivery must therefore be content-aware and byte-verified. The
    // mtime is floored to a whole second: rsync compares seconds, and a
    // sub-second value could round up to the next second and break the
    // equality this fixture relies on.
    for (const rel of ["Info.plist", "macos-arm64/libspoke_connect.a"]) {
      const second = new Date(Math.floor(statSync(join(root, artifact, rel)).mtime.getTime() / 1000) * 1000);
      utimesSync(join(root, bundle, rel), second, second);
    }
    put("artifact/spoke_connectFFI.xcframework.sha256", manifest);
    put("artifact/provenance.json", JSON.stringify({
      sourceHead: invalidProvenance ? "0".repeat(40) : producerSha,
      checkoutRevision: invalidCheckout ? "1".repeat(40) : producerSha,
      manifest, rustcVersion: "rustc fixture",
    }));
    const steps = [...required, ...failures].filter((name) => name !== missing).map((name) => ({
      name,
      status: "completed",
      conclusion: name === cancelledStep ? "cancelled" : failures.includes(name) ? "failure" : "success",
    }));
    put("jobs.json", JSON.stringify({ jobs: [{ name: "xcframework", status: jobStatus, conclusion: jobConclusion ?? conclusion, steps }] }));
    put("metadata", `Xcframework|${invalidSource ? "0".repeat(40) : producerSha}|${status}|${conclusion}`);
    put("bin/gh", `#!/bin/sh\nif [ "$2" = view ]; then\n  case "$*" in *--template*) cat "$FIXTURE_ROOT/metadata" ;; *) cat "$FIXTURE_ROOT/jobs.json" ;; esac\nelif [ "$2" = download ]; then\n  cp -Rp "$FIXTURE_ROOT/artifact/." "$7"\nelse exit 1; fi\n`, { mode: 0o755 });
    if (transferWithoutDelivery) {
      // Transfer boundary stub that reports success without delivering any
      // bytes: the script must refuse via the delivered-byte verification
      // instead of staging the untouched old bundle.
      put("bin/rsync", "#!/bin/sh\nexit 0\n", { mode: 0o755 });
    }
    const result = spawnSync("bash", [script, "123", ...(allowSha === true ? ["--allow-sha", producerSha] : allowSha ? ["--allow-sha", allowSha] : [])], {
      cwd: root, encoding: "utf8", env: { ...env, FIXTURE_ROOT: root, PATH: `${join(root, "bin")}:${process.env.PATH}` },
    });
    const contents = readFileSync(join(root, bundle, "macos-arm64/libspoke_connect.a"), "utf8");
    const recordPath = join(root, "crates/spoke-connect/bindings/swift/xcframework/provenance.json");
    let record;
    try { record = JSON.parse(readFileSync(recordPath, "utf8")); } catch (error) { if (error.code !== "ENOENT") throw error; }
    return { result, contents, record, head, producerSha };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe("xcframework artifact application outcome gate", () => {
  for (const failures of [[freshness], [freshness, smoke], [smoke]]) {
    it(`applies a validated artifact after ${failures.join(" and ")}`, () => {
      const { result, contents, record, producerSha } = runFixture({ failures });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(contents, "new native\n");
      assert.equal(record.xcframework.runConclusion, "failure");
      assert.equal(record.xcframework.headSha, producerSha);
      assert.equal(record.xcframework.allowSha, null);
    });
  }
  for (const [label, options, diagnostic] of [
    ["unrelated failure", { failures: [freshness, "unrelated"] }],
    ["missing prerequisite", { missing: required[0] }],
    ["wrong source", { invalidSource: true }],
    ["invalid manifest", { invalidManifest: true }],
    ["invalid producer provenance", { invalidProvenance: true }],
    ["producer checkout that is not the run head", { invalidCheckout: true }, /checkoutRevision/],
    ["job conclusion does not match the run", { jobConclusion: "success" }, /job status or conclusion does not match run/],
    ["cancelled producer step", { cancelledStep: freshness }, /incomplete or cancelled step/],
    ["incomplete run", { status: "in_progress" }],
    ["cancelled run", { conclusion: "cancelled" }],
  ]) {
    it(`refuses ${label} before bundle mutation`, () => {
      const { result, contents, record } = runFixture(options);
      assert.notEqual(result.status, 0, result.stdout);
      if (diagnostic) assert.match(result.stderr, diagnostic);
      assert.equal(contents, "old native\n");
      assert.equal(record, undefined);
    });
  }
});

describe("cross-head --allow-sha override", () => {
  it("applies a genuine cross-head pin when the producer built the pinned head", () => {
    const { result, contents, record, head, producerSha } = runFixture({
      conclusion: "success", failures: [], advanceHead: true, allowSha: true,
    });
    assert.equal(result.status, 0, result.stderr);
    assert.notEqual(head, producerSha);
    assert.equal(contents, "new native\n");
    assert.equal(record.xcframework.headSha, producerSha);
    assert.equal(record.xcframework.appliedAtHead, head);
    assert.equal(record.xcframework.allowSha, producerSha);
  });

  it("refuses a producer checkout mismatch even with the correct override", () => {
    const { result, contents, record } = runFixture({
      conclusion: "success", failures: [], advanceHead: true, allowSha: true, invalidCheckout: true,
    });
    assert.notEqual(result.status, 0, result.stdout);
    assert.match(result.stderr, /checkoutRevision/);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });

  it("refuses an override naming a different build", () => {
    const { result, contents, record } = runFixture({
      conclusion: "success", failures: [], advanceHead: true, allowSha: "2".repeat(40),
    });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });
});

describe("delivered-byte verification", () => {
  it("refuses a transfer that reports success without delivering the manifest bytes", () => {
    const { result, contents, record } = runFixture({
      conclusion: "success", failures: [], transferWithoutDelivery: true,
    });
    assert.notEqual(result.status, 0, result.stdout);
    assert.match(result.stderr, /did NOT match/);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });
});
