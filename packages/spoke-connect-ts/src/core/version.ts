/**
 * Connect protocol version (not the data `schema_version`).
 *
 * Mirrors `crates/spoke-connect/src/core/mod.rs` `PROTOCOL_VERSION`; protocol
 * version **1** is current. Exchanged in `ConnectHello`.
 */
export const PROTOCOL_VERSION = 1;
