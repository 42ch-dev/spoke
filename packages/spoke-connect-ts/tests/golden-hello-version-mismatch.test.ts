import { describe, expect, it } from "vitest";

import { readFileSync } from "node:fs";

import type { ConnectHello } from "@42ch/spoke-schemas";

import { verifyHelloEd25519 } from "../src/core/hello.js";
import { PROTOCOL_VERSION } from "../src/core/version.js";
import { fromHex } from "./hex.js";

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
