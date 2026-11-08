---
module: connect / native artifact provenance
date: 2026-08-18
last_updated: 2026-11-08
problem_type: tooling_decision
category: tooling-decisions
severity: medium
tags:
  - native-artifacts
  - xcframework
  - provenance
  - source-freshness
  - lfs
  - reproducibility
applies_when:
  - Refreshing committed native carriers
  - Verifying source-to-artifact freshness
---

# Source-bound committed native artifacts

## Context

Committed C, Swift and Go carriers let consumers use the native surfaces directly. Artifact integrity, compiled-source freshness, ABI compatibility and runtime functionality are separate evidence claims. Each claim retains its actual inputs and producer identity.

## Guidance

### Identify the compiled input closure

Record the actual producer source independently of later artifact-only or application-tool commits. The C producer's existing source-identity and dirty-source mechanism includes `spoke-connect`, its C carrier and the actual `spoke-operations`/`spoke-schemas` dependency sources, together with its configured build inputs. Both dirty rejection and revision selection consume the shared source-path list.

A simulated compiler can exercise source-identity decisions in an isolated Git repository. Actual native-source freshness comes from the corresponding real producer and artifact evidence. Unchanged declarations, a passing ABI check or a consumer smoke establishes its own claim rather than source freshness.

### Preserve each carrier's producer record

| Carrier | Existing producer and retained evidence |
|---|---|
| C/C++ | `cpp-connect.yml` produces macOS and Windows artifacts. Each RID keeps its own source revision, target, observed Rust/compiler versions, flags, artifact hashes and both `headerSha256`/`hppSha256`. Apply each RID's files and entry from its own producer artifact. |
| Swift | `xcframework.yml` uses the pinned, locked staged build for the three-slice bundle. The uploaded manifest and producer record bind transferred files to the run's source and observed compiler. The committed-versus-built decision follows upload. |
| Go Darwin | The existing maintainer recipe builds both committed Darwin targets, copies their dylibs and assigns `@rpath/libspoke_connect.dylib`. Retain the local source, toolchain, SDK, flags, full-byte comparisons and applied hashes. |

Both C header hashes are required per RID; missing or mismatching values fail verification. Header byte representation follows the checkout's LF attributes. Native binaries and each RID's metadata remain one source-bound pair; a bundled sibling entry is not evidence for a different RID's build.

Swift static archives and C dynamic carriers use git-lfs. Applied LFS pointer OIDs must agree with the recorded artifact hashes. Go dylibs remain ordinary Git objects for module-proxy consumption. The C import library and provenance are ordinary Git objects.

### Keep the Swift refresh path usable and fail-closed

The committed Swift consumer smoke exercises the committed macOS slice. The staged producer builds and uploads the refresh bundle, then the two-tree verifier compares it with the committed tree. A stale committed carrier can therefore fail freshness while leaving a valid refresh artifact available.

`apply-xcframework-artifact.sh <run-id>` requires the expected completed workflow/job and successful build, manifest, provenance and upload steps. A failed run is usable only when its failed checks are confined to the explicitly named committed freshness and consumer-smoke checks. Other failures, incomplete/cancelled runs or inconsistent outcomes are rejected.

Normal application matches the checkout HEAD to the run's source. Before replacing the bundle, the script validates every transferred manifest entry, binds producer `sourceHead` to the run head, checks the embedded manifest against the transferred manifest and requires an observed compiler value. The sidecar records the actual run conclusion, source and application head. An intentional explicit source override is recorded separately.

A second download of the same build can verify transfer and application identity. Independent reproducibility requires another real source build with equivalent inputs. The producer's failing old-tree comparison, the applied artifact's passing integrity check and later delivery CI retain their distinct results.

### Compare independent builds with identical complete inputs

A complete build input includes the source, lockfile/dependency basis, selected toolchain, target, SDK, effective Cargo/rustc options and absolute output path. On Darwin the initial `LC_ID_DYLIB` can contain that path; different path lengths can alter load-command space and section offsets as well as UUIDs. Reassigning the final install name alone need not make differently built files identical.

For a focused reproducibility check, retain the first output, clean the task-owned build directory, and compile again at the exact same path with the same inputs. Compare complete raw outputs, then compare the separately processed first output with the applied second output. A genuinely different old same-architecture carrier supplies a meaningful mismatch control. Full-byte comparisons preserve UUIDs, signatures and sections.

Honor the repository's local nightly convention and effective options. `-Zno-embed-metadata` is a Cargo option and requires a compatible selected nightly; its role is distinct from rustc `build.rustflags`. Record the actual selected versions and options. The documented Go recipe's lockfile basis and any use of `--locked` remain explicit producer facts.

Producer-level normalization of `Info.plist` `AvailableLibraries` ordering stabilizes the emitted bundle before manifest generation. System bash3.2 scripts expand an empty array safely under `set -u` with `${arr[@]+"${arr[@]}"}`.

## Why This Matters

Source-bound producer records explain which implementation consumers receive. Required committed-carrier checks preserve drift detection, while upload-before-decision and the guarded application policy make a legitimate refresh possible. Independent cold builds and whole-file mismatch controls establish reproducibility within their declared input boundary.

## When to Apply

- A compiled dependency or FFI implementation change affects committed carriers.
- A required freshness check reports drift and a genuine refresh artifact is available.
- A native comparison needs to distinguish changed inputs from same-input nondeterminism.

## References

- [Native-binding channels](../../specs/connect-binding-channels.md)
- [Maintainer carrier procedures](../../../CONTRIBUTING.md)
- Source: `.github/workflows/cpp-connect.yml`, `.github/workflows/xcframework.yml`, `tooling/connect/cpp-build.mjs`, `apply-xcframework-artifact.sh`, `verify-xcframework-drift.sh`.
