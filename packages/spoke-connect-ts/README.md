# @42ch/spoke-connect-ts

SPOKE connect client library (TypeScript) — peer identity derivation, Ed25519 hello signing, RFC 8785 JCS canonicalization, one-JSON-per-message WebSocket framing, and the pure session-core port (sequence, correlation, dispatch gate, nonce store, allowlist).

Workspace-private package: the version tracks the monorepo lockstep SemVer (asserted by `verify:version`, bumped by `release:bump`). The transport is a direct WebSocket ordered reliable stream using plain JSON + WebSocket framing; transport and crypto are dependency-light by design.

## What it provides

- **Identity** — `derivePeerIdFromEd25519Pubkey`: protobuf PublicKey → identity multihash `0x00` → base58btc. Ported from `tooling/connect-identity-proof/proof.mjs`; the normative formula lives in `.mstar/specs/spoke-connect.md` § Identity binding.
- **Crypto** — Ed25519 sign/verify over raw 32-byte keys, WebCrypto primary with an `@noble/ed25519` fallback on the same code path; base64url without padding.
- **JCS** — `canonicalHelloBytes(peerId, nonce, host)`: RFC 8785 canonicalization of the signed hello object (`{protocol_version, peer_id, nonce, host}`) via the pinned `canonicalize` package. Absent optional manifest members are omitted; only present members appear in the canonical object.
- **Session core** (`src/core/`) — behavior port of `crates/spoke-connect/src/core/`: `OutboundSequence` / `InboundSequence` (start 0, exhaustion instead of wrap past `2^53−1`), response correlation (session_id / sequence / request_id echo), op dispatch gate (capability ⊆ negotiated, unknown op fails closed), per-sender `NonceStore`, fail-closed allowlist, thin `Session` helper, `PROTOCOL_VERSION`.
- **Node client** (`src/node/`) — `connectClient({ url, identity, manifest, remotePubkey, allowlist })`: dials a WebSocket, performs the signed hello exchange, validates the session snapshot (peer binding, `initial_sequence` 0), then routes correlated invokes by `request_id` with bounded waits. Node-only because it uses `ws`; the isomorphic `src/` modules stay browser-swappable.

**Golden-vector parity:** tests assert peer_id / JCS bytes / signature byte-identical to the Rust golden vectors in `crates/spoke-connect/src/core/` (`peer_id.rs`, `hello_crypto.rs`), with constants redeclared in `src/golden.ts` (provenance comments included).

## Usage

Monorepo-internal (workspace-private; subpaths resolve via the package `exports` map):

```ts
import { derivePeerIdFromEd25519Pubkey } from "@42ch/spoke-connect-ts";
import { connectClient } from "@42ch/spoke-connect-ts/node";

const seed = new TextEncoder().encode("..."); // 32-byte Ed25519 seed
const remotePubkey = /* the server's 32-byte Ed25519 public key */;

const client = await connectClient({
  url: "ws://127.0.0.1:8080",
  identity: { seed },
  manifest: {
    capabilities: ["spoke-baseline"],
    extensions: {},
    host_id: "host_primary",
    namespaces: ["toy_world"],
    roles: ["data-store"],
    schema_version: 1,
  },
  remotePubkey,
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
});

const response = await client.invoke("check", { /* op payload */ });
client.close();
```

Core helpers are importable without the client: `signHelloEd25519` / `verifyHelloEd25519`, `OutboundSequence`, `checkResponseCorrelation`, `dispatchAllowed`, `NonceStore`, `isAllowlisted`, `Session` — all from `@42ch/spoke-connect-ts`.

## Test

From the repo root:

```bash
pnpm run test:connect-ts
```

From this directory:

```bash
pnpm test
pnpm run typecheck
```

The two-node interop test (`tests/two-node.test.ts`) runs an in-process `ws` server and client over `127.0.0.1:<ephemeral>` with bounded waits only. CI runs the suite on Node 20.x — Node ≥ 20.19 takes the WebCrypto Ed25519 path; older patches fall back to `@noble/ed25519`. The package engine floor is Node ≥ 20.19.0 (`@noble/hashes` floor, and the first Node line that accepts WebCrypto Ed25519).

## Scope

- Workspace-private (`"private": true`) package shipping inside the monorepo.
- The package consumes the existing connect schemas as-is (`@42ch/spoke-schemas`, workspace dependency, types only); the schema inventory is unchanged.
- Envelope-level interop over any ordered reliable stream; framing is direct WebSocket per `.mstar/specs/spoke-connect.md` § Transport framing.
- The client targets the direct ordered-stream transport.
- `connectClient` lives in the Node `src/node/` subpath (uses `ws`); the isomorphic `src/` modules are browser-swappable with the native WebSocket.

## Publish guidance

The package is workspace-private at the current version (`"private": true`); the publish strategy, staging, and triggers are defined in `.mstar/specs/connect-publish-strategy.md` (repository-internal reference).

- **Entry points** — the package exposes two subpaths: `.` (isomorphic core: identity, crypto, JCS, session core) and `./node` (the Node `connectClient`, which depends on `ws`). Browser consumers import `.` only.
- **License** — declared Apache-2.0 via the `license` field, mirroring the published sibling packages (`@42ch/spoke-schemas`, `@42ch/spoke-operations`); the authoritative license text lives at the repository root (`LICENSE`).
- **Versioning** — lockstep SemVer with the monorepo (`verify:version`, `release:bump`); `@42ch/spoke-schemas` resolves at the same version from npm (workspace `workspace:*` is rewritten at pack time).
- **Installation** — the package installs from the workspace at the current version. When the Stage 1 publish defined in the strategy document executes, `@42ch/spoke-connect-ts` publishes to npm and consumers install it from the registry at the same lockstep version as `@42ch/spoke-schemas`. The Stage 1 release procedure lives in `.mstar/specs/connect-publish-strategy.md` (repository-internal).
