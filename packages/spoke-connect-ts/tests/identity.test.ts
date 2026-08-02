import { describe, expect, it } from "vitest";

import { derivePeerIdFromEd25519Pubkey } from "../src/identity.js";
import { GOLDEN_PEER_ID, GOLDEN_PUBKEY } from "../src/golden.js";

describe("derivePeerIdFromEd25519Pubkey", () => {
  it("derives the golden peer_id from the golden public key", () => {
    expect(derivePeerIdFromEd25519Pubkey(GOLDEN_PUBKEY)).toBe(GOLDEN_PEER_ID);
  });

  it("produces libp2p Ed25519-shaped peer ids (12D3KooW prefix, fixed length)", () => {
    const a = derivePeerIdFromEd25519Pubkey(new Uint8Array(32).fill(3));
    expect(a.startsWith("12D3KooW")).toBe(true);
    expect(a.length).toBe(GOLDEN_PEER_ID.length);
  });

  it("derives distinct peer ids from distinct keys", () => {
    const a = derivePeerIdFromEd25519Pubkey(new Uint8Array(32).fill(1));
    const b = derivePeerIdFromEd25519Pubkey(new Uint8Array(32).fill(2));
    expect(a).not.toBe(b);
  });

  it("rejects non-32-byte inputs", () => {
    expect(() => derivePeerIdFromEd25519Pubkey(new Uint8Array(31))).toThrow(
      "pubkey must be 32 bytes",
    );
  });
});
