# ADR: Publish channels for the connect client surfaces

Status: implemented — each connect surface has exactly one channel, one tag gate and one release workflow

Date: 2026-08-02

Scope: connect publish staging, the registry-backed versus git-based channel split, registry auth, and the transport options that were not adopted

## Problem

Connect ships one Rust reference crate, one TypeScript client and six language bindings, and each language ecosystem resolves dependencies differently. Without a recorded split, every surface could land on a different feed, a different version source and a different auth model, so an integrator pinning one `X.Y.Z` would face an inconsistent install story, and release automation would grow a publish job per opinion.

The forces in conflict: registry packages give integrators a normal resolver experience and carry the version in package metadata, but each feed needs credentials, publisher registration and a publish job; git resolution needs nothing beyond a tag, but pushes version discovery, native-artifact layout and (for C/C++) git-lfs handling onto the consumer. Swift and Go cannot be published to the feeds the other languages use, and the C/C++ channel has no version-bearing manifest to publish at all.

## Decision

Registry-backed distribution where the ecosystem has a registry the project controls, git-based distribution where the ecosystem resolves from the repository itself:

| Language | Channel | Coordinates |
|---|---|---|
| C# | GitHub Packages NuGet | `42ch.Spoke.Connect` at `https://nuget.pkg.github.com/42ch-dev/index.json` |
| Kotlin | GitHub Packages Maven | `dev.42ch:spoke-connect` at `maven.pkg.github.com/42ch-dev/spoke` |
| Python | PyPI with Trusted Publishing | `spoke-connect` |
| Swift | Swift Package Manager over git | root `Package.swift`, product `SpokeConnect` |
| Go | Go modules over git | root `go.mod`, module `github.com/42ch-dev/spoke` |
| C / C++ | git, committed headers plus platform carriers | `include/spoke_connect.h`, `include/spoke_connect.hpp`, `native/<rid>/` |

NuGet and Maven share the GitHub Packages registry family, so one `GITHUB_TOKEN` with `packages: write` covers both; PyPI uses OIDC Trusted Publishing with the publisher registered against repository `42ch-dev/spoke` and workflow `release.yml`.

All channels share one gate: stable `vX.Y.Z` and non-`-rc.` prerelease tags publish, `-rc.` tags create GitHub pre-releases only, and `.github/workflows/release.yml` stays the sole top-level publish workflow because Trusted Publishing OIDC binds to that filename. Versions stay in monorepo lockstep: registry packages carry the repository `X.Y.Z` in their metadata, git-based consumers resolve the matching tag, and the C/C++ channel adds no version-bearing manifest.

Release re-runs must be safe. The PyPI and Maven lanes pre-check the registry for the full expected artifact set at the tag version and skip build and publish when it is complete; only an unconditional re-probe decides whether the job is green, and registry doubt (network error, HTTP 401/403/5xx, malformed JSON, unexpected payload) fails the job instead of reporting success.

## Alternatives considered

**nuget.org as the C# feed.** Lost to GitHub Packages for now: one feed inside the same credential model as the rest of the repository is enough until integrator demand justifies a second feed, and a mirror doubles publisher and workflow surface.

**Maven Central as the Kotlin feed.** Lost for the same reason, but the coordinates stay `dev.42ch:...` — the reverse-DNS form Maven Central requires — so a later mirror remains a publishing change rather than a coordinate migration.

**GitHub Packages for Swift, Go and Python.** Rejected: Swift Package Manager resolves git-url dependencies from a repository-root manifest only, Go modules resolve from repository tags, and PyPI is the Python registry; none of those ecosystems maps onto GitHub Packages without a second packaging layer that no consumer asked for.

**vcpkg, Conan and Release archive assets for C/C++.** Rejected: the channel ships committed headers and platform carriers resolved from the tag, so a package manager or archive would add a second artifact shape and a per-platform packaging burden without a consumer requirement.

**Independent per-surface SemVer before 1.0.** Rejected: integrators pin one version across schemas, operations and connect surfaces, and `release:bump` / `verify:version` already cover the manifests; a split would fork the bump scripts and the changelog story for little gain. The revisit trigger is a binding gaining a materially different release cadence or a mirror demand, recorded before any channel changes.

**WebTransport as the default Path A transport.** Deferred: browser baseline availability arrived in 2026-03, but the Node-side implementation stays flag-gated and experimental, the reference spike has no WebTransport transport, and the one-envelope-per-message contract maps one-to-one onto WebSocket — so there is no adapter-level framing work that only WebTransport can do. It stays a future framing option under transport-adapter ownership, and the trigger is a product host that requires HTTP/3 WebTransport plus a CI-able browser test plan.

**js-libp2p in the default dependency graph.** Rejected: envelope-level interop with the reference needs only the same hello, `peer_id` and session rules over an ordered stream, while direct libp2p-network membership needs the whole Noise/yamux mesh. The `@libp2p/*` tree is a deep supply-chain and bundle surface, the inverse of the thin helper packages the protocol repo publishes, and Noise/multistream interop with rust-libp2p 0.57 needs version pinning and periodic re-verification. A product that needs the shared mesh gets an optional companion package or a consumer-repo adapter.

## Consequences

- `.github/workflows/release.yml` grows sibling publish jobs rather than a second workflow; OIDC publisher registrations must keep pointing at that filename.
- Every channel needs a verifiable expected-artifact set, otherwise a re-run either duplicates artifacts or reports green on a partial publish.
- Git-based channels depend on tag hygiene; the C/C++ carrier libraries are git-lfs objects, so consumers run `git lfs install` once.
- The C# package is session core only; a WebSocket client would be a separate, later package.
- The TypeScript default export graph stays thin, and js-libp2p stays a documented fallback rather than a shipped default.
- WebTransport, nuget.org, Maven Central, vcpkg/Conan and independent binding versioning each remain recorded non-goals with an explicit trigger, not silent exclusions.
