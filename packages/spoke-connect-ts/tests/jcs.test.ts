import { describe, expect, it } from "vitest";

import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

import { canonicalHelloBytes } from "../src/jcs.js";
import {
  GOLDEN_JCS_HEX,
  GOLDEN_NONCE,
  GOLDEN_PEER_ID,
  goldenManifest,
} from "../src/golden.js";
import { fromHex, toHex } from "./hex.js";

describe("canonicalHelloBytes (RFC 8785 JCS)", () => {
  it("canonicalizes the golden signed object to the golden bytes (264)", () => {
    const bytes = canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, goldenManifest());
    expect(bytes.length).toBe(264);
    expect(toHex(bytes)).toBe(GOLDEN_JCS_HEX);
  });

  it("is deterministic across host member insertion order", () => {
    const hostA: HostCapabilityManifest = goldenManifest();
    const manifest = goldenManifest();
    const hostB: HostCapabilityManifest = {
      schema_version: manifest.schema_version,
      roles: manifest.roles,
      namespaces: manifest.namespaces,
      capabilities: manifest.capabilities,
      extensions: manifest.extensions,
      host_id: manifest.host_id,
    };
    const a = canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, hostA);
    const b = canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, hostB);
    expect(a).toEqual(b);
  });

  it("covers exactly the four signed members", () => {
    const bytes = canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, goldenManifest());
    const parsed = JSON.parse(new TextDecoder().decode(bytes)) as Record<
      string,
      unknown
    >;
    expect(Object.keys(parsed).sort()).toEqual([
      "host",
      "nonce",
      "peer_id",
      "protocol_version",
    ]);
  });

  it("omits absent optional members — never emits null", () => {
    const canonical = new TextDecoder().decode(
      canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, goldenManifest()),
    );
    expect(canonical).not.toContain("authority");

    // A manifest WITH authority canonicalizes to different bytes: an
    // explicit null would too — the omit-not-null rule is signature-binding.
    const withAuthority: HostCapabilityManifest = {
      ...goldenManifest(),
      authority: { scope_key: "golden-scope" },
    };
    const withAuthorityBytes = canonicalHelloBytes(
      GOLDEN_PEER_ID,
      GOLDEN_NONCE,
      withAuthority,
    );
    expect(toHex(withAuthorityBytes)).not.toBe(GOLDEN_JCS_HEX);
  });

  it("golden hex decodes to the same bytes the module produces", () => {
    const bytes = canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, goldenManifest());
    expect(bytes).toEqual(fromHex(GOLDEN_JCS_HEX));
  });
});
