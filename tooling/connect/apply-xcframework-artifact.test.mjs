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
const smoke = "Fail on committed Swift consumer smoke";
const digest = (text) => createHash("sha256").update(text).digest("hex");
const bundle = "crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework";

function runFixture({
  failures = [smoke], missing, invalidSource, invalidManifest, invalidProvenance, invalidCheckout,
  status = "completed", conclusion = "failure",
  jobStatus = "completed", jobConclusion, cancelledStep, advanceHead, allowSha, transferWithoutDelivery,
  gateJobs = [], mergeCheckout, driftCheckout, siblingDrift,
  graph, workerConclusion, workerAttempts, assembleAttempt = 1, assembleStepFailures = [],
  assertFails, noProducer, producerExpired, noFinal, finalExpired,
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
    put(".github/workflows/xcframework.yml", [
      "toolchain: 1.98.1",
      "targets: aarch64-apple-darwin",
      "targets: aarch64-apple-ios",
      "targets: aarch64-apple-ios-sim",
      "targets: x86_64-apple-ios",
      "targets: aarch64-apple-darwin",
      'XCFRAMEWORK_LOCKED: "1"',
      "",
    ].join("\n"));
    put(`${bundle}/Info.plist`, "old plist\n");
    put(`${bundle}/macos-arm64/libspoke_connect.a`, "old native\n");
    git("init", "-q");
    git("add", ".");
    git("commit", "-qm", "fixture");
    // A pull-request producer builds GitHub's virtual merge: a commit whose
    // build inputs equal the declared head, which then sits on top of it.
    const checkoutBase = git("rev-parse", "HEAD");
    if (mergeCheckout) {
      put("NOTES.md", "base branch note\n");
      git("add", ".");
      git("commit", "-qm", "declare the head above the merge checkout");
    }
    if (driftCheckout) {
      put("crates/spoke-connect/src/drift.rs", "drift\n");
      git("add", ".");
      git("commit", "-qm", "declare the head above a drifting checkout");
    }
    if (siblingDrift) {
      put(siblingDrift, "drift\n");
      git("add", ".");
      git("commit", "-qm", "declare the head above a drifting sibling crate");
    }
    const producerSha = git("rev-parse", "HEAD");
    const checkoutRevision = mergeCheckout || driftCheckout || siblingDrift ? checkoutBase : producerSha;
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
      checkoutRevision: invalidCheckout ? "1".repeat(40) : checkoutRevision,
      manifest, rustcVersion: "rustc fixture",
    }));
    const steps = [...required, ...failures].filter((name) => name !== missing).map((name) => ({
      name,
      status: "completed",
      conclusion: name === cancelledStep ? "cancelled" : failures.includes(name) ? "failure" : "success",
    }));
    const done = (name, extra = {}) => ({ name, status: "completed", conclusion: "success", run_attempt: 1, ...extra });
    const step = (name, conclusion = "success") => ({ name, status: "completed", conclusion });
    const graphWorkers = ["host", "slice-macos", "slice-ios-arm64", "slice-ios-arm64-sim", "slice-ios-x86_64-sim"]
      .map((name, index) => done(name, {
        conclusion: workerConclusion?.[name] ?? "success",
        run_attempt: workerAttempts?.[index] ?? 1,
        steps: [],
      }));
    const assembleRequired = [
      "Assemble xcframework from slices",
      "Write xcframework hash manifest",
      "Record producer record",
      "Upload xcframework artifact",
    ];
    const graphJobs = graph === undefined ? [] : [
      done("changes"),
      ...graphWorkers,
      done("assemble", {
        conclusion: graph.assemble ?? "success",
        run_attempt: assembleAttempt,
        steps: assembleRequired.map((name) => step(name, assembleStepFailures.includes(name) ? "failure" : "success")),
      }),
      done("xcframework", {
        conclusion,
        steps: [
          step("Assert gate inputs and worker results", assertFails ? "failure" : "success"),
          step("Run committed Swift consumer smoke"),
          step("Fail on committed Swift consumer smoke", conclusion === "failure" ? "failure" : "skipped"),
          step("Fail on required-check assertion", conclusion === "failure" && assertFails ? "failure" : "skipped"),
        ],
      }),
    ];
    put("jobs.json", JSON.stringify({ jobs: graph === undefined ? [...gateJobs, { name: "xcframework", status: jobStatus, conclusion: jobConclusion ?? conclusion, steps }] : graphJobs }));
    if (graph !== undefined) {
      const producerName = `spoke-connect-xcframework-producer-123-${assembleAttempt}`;
      const finalName = `spoke-connect-xcframework-123-${assembleAttempt}`;
      put("artifacts.json", JSON.stringify({ artifacts: [
        ...(noProducer ? [] : [{ id: 501, name: producerName, expired: Boolean(producerExpired) }]),
        ...(noFinal ? [] : [{ id: 502, name: finalName, expired: Boolean(finalExpired) }]),
      ] }));
      put(`artifact/${producerName}/producer.json`, JSON.stringify({
        validated: true, runId: 123, runAttempt: assembleAttempt, workerAttempt: workerAttempts?.[0] ?? 1,
        workers: {
          host: { result: graphWorkers[0].conclusion, attempt: graphWorkers[0].run_attempt, "artifact-name": `xc-host-123-${graphWorkers[0].run_attempt}`, "artifact-id": "601" },
          "slice-macos": { result: graphWorkers[1].conclusion, attempt: graphWorkers[1].run_attempt, "artifact-name": `xc-slice-macos-arm64-123-${graphWorkers[1].run_attempt}`, "artifact-id": "602" },
          "slice-ios-arm64": { result: graphWorkers[2].conclusion, attempt: graphWorkers[2].run_attempt, "artifact-name": `xc-slice-ios-arm64-123-${graphWorkers[2].run_attempt}`, "artifact-id": "603" },
          "slice-ios-arm64-sim": { result: graphWorkers[3].conclusion, attempt: graphWorkers[3].run_attempt, "artifact-name": `xc-slice-ios-arm64-simulator-123-${graphWorkers[3].run_attempt}`, "artifact-id": "604" },
          "slice-ios-x86_64-sim": { result: graphWorkers[4].conclusion, attempt: graphWorkers[4].run_attempt, "artifact-name": `xc-slice-ios-x86_64-simulator-123-${graphWorkers[4].run_attempt}`, "artifact-id": "605" },
        },
        finalArtifact: { id: 502, name: finalName },
      }));
      put(`artifact/${finalName}/spoke_connectFFI.xcframework/Info.plist`, plist);
      put(`artifact/${finalName}/spoke_connectFFI.xcframework/macos-arm64/libspoke_connect.a`, native);
      put(`artifact/${finalName}/spoke_connectFFI.xcframework.sha256`, manifest);
      put(`artifact/${finalName}/provenance.json`, JSON.stringify({
        sourceHead: producerSha, checkoutRevision, manifest, rustcVersion: "rustc fixture",
        sourceRevision: producerSha, recipeRevision: producerSha,
      }));
    }
    put("metadata", `Xcframework|${invalidSource ? "0".repeat(40) : producerSha}|${status}|${conclusion}`);
    put("bin/gh", [
      "#!/bin/sh",
      "case \"$1\" in",
      "  api)",
      "    case \"$2\" in",
      "      *jobs*) cat \"$FIXTURE_ROOT/jobs.json\" ;;",
      "      *artifacts*) cat \"$FIXTURE_ROOT/artifacts.json\" ;;",
      "      *) exit 1 ;;",
      "    esac ;;",
      "  run)",
      "    if [ \"$2\" = view ]; then",
      "      case \"$*\" in *--template*) cat \"$FIXTURE_ROOT/metadata\" ;; *) cat \"$FIXTURE_ROOT/jobs.json\" ;; esac",
      "    elif [ \"$2\" = download ]; then",
      "      dir=\"\"; name=\"\"; prev=\"\"",
      "      for arg in \"$@\"; do",
      "        [ \"$prev\" = \"--dir\" ] && dir=\"$arg\"",
      "        [ \"$prev\" = \"--name\" ] && name=\"$arg\"",
      "        case \"$arg\" in --dir=*) dir=\"${arg#--dir=}\" ;; --name=*) name=\"${arg#--name=}\" ;; esac",
      "        prev=\"$arg\"",
      "      done",
      "      if [ \"$name\" = \"spoke-connect-xcframework\" ]; then",
      "        cp -Rp \"$FIXTURE_ROOT/artifact/.\" \"$dir\"",
      "      elif [ -d \"$FIXTURE_ROOT/artifact/$name\" ]; then",
      "        mkdir -p \"$dir\"",
      "        cp -Rp \"$FIXTURE_ROOT/artifact/$name\" \"$dir/\"",
      "      else",
      "        echo \"no artifact matches any of the names or patterns provided\" >&2",
      "        exit 1",
      "      fi",
      "    else exit 1; fi ;;",
      "  *) exit 1 ;;",
      "esac",
      "",
    ].join("\n"), { mode: 0o755 });
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

describe("workflow gate jobs beside the producer", () => {
  const gate = { name: "changes", status: "completed", conclusion: "success" };

  it("applies a validated artifact when the changes path-filter gate also ran", () => {
    const { result, contents, record, producerSha } = runFixture({ gateJobs: [gate] });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(contents, "new native\n");
    assert.equal(record.xcframework.headSha, producerSha);
    assert.equal(record.xcframework.sourceRevision, "unknown");
    assert.equal(record.xcframework.recipeRevision, "unknown");
  });

  it("refuses a gate job that did not succeed", () => {
    const { result, contents, record } = runFixture({ gateJobs: [{ ...gate, conclusion: "failure" }] });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });

  it("refuses a second job that is not a gate", () => {
    const { result, contents, record } = runFixture({ gateJobs: [{ ...gate, name: "extra" }] });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });
});

describe("pull-request merge checkout", () => {
  it("applies an artifact whose checkout is a merge with identical build inputs", () => {
    const { result, contents, record, producerSha } = runFixture({ mergeCheckout: true });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(contents, "new native\n");
    assert.equal(record.xcframework.headSha, producerSha);
    assert.notEqual(record.xcframework.producerCheckout, producerSha);
  });

  it("refuses a merge checkout that drifts in a build input", () => {
    const { result, contents, record } = runFixture({ driftCheckout: true });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });

  // spoke-connect path-depends on both sibling crates, so a build input that
  // differs only there still changes the linked artifact.
  for (const path of ["crates/spoke-schemas/src/drift.rs", "crates/spoke-operations/src/drift.rs"]) {
    it(`refuses a merge checkout that drifts only in ${path.split("/").slice(0, 2).join("/")}`, () => {
      const { result, contents, record } = runFixture({ siblingDrift: path });
      assert.notEqual(result.status, 0, result.stdout);
      assert.equal(contents, "old native\n");
      assert.equal(record, undefined);
    });
  }

  it("refuses a producer checkout that is not in this checkout", () => {
    const { result, contents, record } = runFixture({ invalidCheckout: true });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });
});

describe("xcframework artifact application outcome gate", () => {
  for (const failures of [[smoke]]) {
    it(`applies a validated artifact after ${failures.join(" and ")}`, () => {
      const { result, contents, record, producerSha } = runFixture({ failures });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(contents, "new native\n");
      assert.equal(record.xcframework.runConclusion, "failure");
      assert.equal(record.xcframework.headSha, producerSha);
      assert.equal(record.xcframework.allowSha, null);
    });
  }
  for (const [label, options] of [
    ["unrelated failure", { failures: [smoke, "unrelated"] }],
    ["missing prerequisite", { missing: required[0] }],
    ["wrong source", { invalidSource: true }],
    ["invalid manifest", { invalidManifest: true }],
    ["invalid producer provenance", { invalidProvenance: true }],
    ["job conclusion does not match the run", { jobConclusion: "success" }],
    ["cancelled producer step", { cancelledStep: smoke }],
    ["incomplete run", { status: "in_progress" }],
    ["cancelled run", { conclusion: "cancelled" }],
  ]) {
    it(`refuses ${label} before bundle mutation`, () => {
      const { result, contents, record } = runFixture(options);
      assert.notEqual(result.status, 0, result.stdout);
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
    assert.equal(contents, "old native\n");
    assert.equal(record, undefined);
  });
});

describe("split producer graph", () => {
  it("applies a validated rebuild artifact resolved through producer.json", () => {
    const { result, contents, record, producerSha } = runFixture({ graph: {}, conclusion: "success", failures: [] });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(contents, "new native\n");
    assert.equal(record.xcframework.headSha, producerSha);
    assert.equal(record.xcframework.artifact, "spoke-connect-xcframework-123-1");
    assert.equal(record.xcframework.sourceRevision, producerSha);
    assert.equal(record.xcframework.recipeRevision, producerSha);
    assert.deepEqual(record.xcframework.declaredRecipe.targets,
      ["aarch64-apple-darwin", "aarch64-apple-ios", "aarch64-apple-ios-sim", "x86_64-apple-ios"]);
  });

  it("applies a rebuild artifact when only the committed smoke failed", () => {
    const { result, contents, record, producerSha } = runFixture({ graph: {}, conclusion: "failure", failures: [] });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(contents, "new native\n");
    assert.equal(record.xcframework.headSha, producerSha);
    assert.equal(record.xcframework.runConclusion, "failure");
  });

  it("refuses mixed worker attempts", () => {
    const { result, contents } = runFixture({ graph: {}, conclusion: "success", failures: [], workerAttempts: [1, 2, 1, 1, 1] });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
  });

  it("refuses mixed worker conclusions", () => {
    const { result, contents } = runFixture({ graph: {}, conclusion: "success", failures: [], workerConclusion: { "slice-macos": "failure" } });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
  });

  it("refuses a run whose workers were all skipped", () => {
    const skipped = Object.fromEntries(["host", "slice-macos", "slice-ios-arm64", "slice-ios-arm64-sim", "slice-ios-x86_64-sim"].map((name) => [name, "skipped"]));
    const { result, contents } = runFixture({ graph: { assemble: "skipped" }, conclusion: "success", failures: [], workerConclusion: skipped });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
  });

  it("refuses a missing producer record artifact", () => {
    const { result, contents } = runFixture({ graph: {}, conclusion: "success", failures: [], noProducer: true });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
  });

  it("refuses an expired final artifact", () => {
    const { result, contents } = runFixture({ graph: {}, conclusion: "success", failures: [], finalExpired: true });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
  });

  it("refuses an assemble job that skipped its upload step", () => {
    const { result, contents } = runFixture({ graph: {}, conclusion: "success", failures: [], assembleStepFailures: ["Upload xcframework artifact"] });
    assert.notEqual(result.status, 0, result.stdout);
    assert.equal(contents, "old native\n");
  });
});
