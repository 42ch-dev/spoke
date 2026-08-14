import { describe, expect, it } from "vitest";

import { readFileSync } from "node:fs";

import type { ConnectHello } from "@42ch/spoke-schemas";

import { verifyHelloEd25519 } from "../src/core/hello.js";
import { PROTOCOL_VERSION } from "../src/core/version.js";
import { fromHex } from "./hex.js";

/**
 * Cross-language golden vector: the golden identity advertising
 * `protocol_version` 2 while the core `PROTOCOL_VERSION` is 1 is a
 * mixed-version hello — the version gate is `verifyHelloEd25519` step 1,
 * BEFORE signature verification, so the wire hello MUST reject with
 * `code: "protocol_version_mismatch"` regardless of its (object-stale,
 * never-consulted) pinned signature.
 *
 * The fixture is a byte-identical registered copy of the SSOT under
 * `crates/spoke-connect/tests/fixtures/` (sync gate:
 * `tooling/connect/golden-vector-sync.mjs`); the Rust side consumes the SSOT
 * directly in `crates/spoke-connect/tests/golden_hello_version_mismatch.rs`
 * and asserts the same outcome — `CoreError::ProtocolVersionMismatch` — on
 * the same bytes: the cross-language parity proof for the shipped
 * version-first gate.
 */

interface GoldenHelloVersionMismatchFixture {
  version: number;
  seed_hex: string;
  pubkey_hex: string;
  peer_id: string;
  hello: ConnectHello;
}

const fixtureUrl = new URL(
  "./fixtures/golden-hello-version-mismatch.json",
  import.meta.url,
);
const fixture: GoldenHelloVersionMismatchFixture = JSON.parse(
  readFileSync(fixtureUrl, "utf8"),
);

describe("golden-hello-version-mismatch (cross-language golden vector)", () => {
  it("rejects the pinned mixed-version hello with protocol_version_mismatch", async () => {
    // The fixture must actually be a mismatch — the golden vector is only
    // meaningful while the core stays at version 1.
    expect(fixture.hello.protocol_version).not.toBe(PROTOCOL_VERSION);

    const publicKey = fromHex(fixture.pubkey_hex);
    await expect(
      verifyHelloEd25519(publicKey, fixture.peer_id, fixture.hello),
    ).rejects.toThrowError(
      expect.objectContaining({ code: "protocol_version_mismatch" }),
    );
  });

  it("pins a schema-conformant wire hello with the golden identity", () => {
    expect(fixture.version).toBe(1);
    expect(fixture.hello.protocol_version).toBe(2);
    expect(fixture.hello.peer_id).toBe(fixture.peer_id);
    expect(fixture.hello.host.host_id).toBe("golden-host");
    expect(fixture.hello.nonce.length).toBeGreaterThanOrEqual(16);
    expect(typeof fixture.hello.signature).toBe("string");
  });
});
