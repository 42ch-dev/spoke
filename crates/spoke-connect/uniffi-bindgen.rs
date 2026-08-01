//! `uniffi-bindgen` CLI entry point for this crate (uniffi 0.32 pattern).
//!
//! uniffi no longer ships an installable `uniffi-bindgen` binary for the
//! 0.32 line; the documented approach is a crate-local bin that forwards to
//! `uniffi::uniffi_bindgen_main`. Build only with the `bindgen-cli` feature:
//!
//! ```text
//! cargo run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
//!   generate --library target/debug/libspoke_connect.dylib \
//!   --language swift --out-dir crates/spoke-connect/bindings/swift/generated
//! ```
//!
//! The library must be built first with `--features ffi` so the cdylib
//! carries the exported-surface metadata.

fn main() {
    uniffi::uniffi_bindgen_main()
}
