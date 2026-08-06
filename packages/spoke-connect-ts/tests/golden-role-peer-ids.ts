import { getPublicKeyEd25519 } from "../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../src/identity.js";

/** Deterministic subject role seed `[7u8; 32]` — same as Rust unit tests. */
export function subjectPeerIdFromRoleSeed(): string {
  return derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(new Uint8Array(32).fill(7)),
  );
}

/** Deterministic audience role seed `[8u8; 32]` — same as Rust unit tests. */
export function audiencePeerIdFromRoleSeed(): string {
  return derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(new Uint8Array(32).fill(8)),
  );
}
