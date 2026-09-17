//! Record-layout report for the C ABI gate.
//!
//! The C header `crates/spoke-connect/bindings/cpp/include/spoke_connect.h`
//! declares the records and callback tables this carrier crosses, and the
//! mirrors here are hand-written to match them. Names are compared by
//! `tooling/connect/cpp-symbol-check.mjs`; the *layouts* are what that gate
//! cannot see from the declarations alone. This module is the Rust half of that
//! comparison: it reports, for every `#[repr(C)]` record the header declares,
//! the size, the alignment and the offset of each field, and the gate turns
//! each reported pair into a `sizeof` / `_Alignof` / `offsetof` assertion
//! compiled against the header. A drift on either side then fails the gate
//! instead of silently changing the ABI.
//!
//! The report runs under `cargo test -p spoke-connect-capi --lib abi_layout --
//! --nocapture`, which the gate drives itself; it is a verification surface and
//! never part of the carrier's exported ABI.

use std::mem::{align_of, offset_of, size_of};

use crate::remote_adapter::SpokeConnectTransportTable;
use crate::responder::{
    SpokeConnectPeerKey, SpokeConnectPortsHandlerTable, SpokeConnectToolHandlerTable,
};
use crate::{
    SpokeConnectBuffer, SpokeConnectError, SpokeConnectForeignBuffer, SpokeConnectForeignError,
    SpokeConnectOptionalBuffer, SpokeConnectOptionalU64, SpokeConnectSlice,
};

/// Reports one record as `SPOKE_CONNECT_ABI_LAYOUT <name> size=<n> align=<n>
/// <field>=<offset> …`. Field names are literals because they are the header's
/// member names: the gate matches the two by name.
macro_rules! report {
    ($name:literal, $record:ty, $fields:expr) => {
        println!(
            "SPOKE_CONNECT_ABI_LAYOUT {} size={} align={} {}",
            $name,
            size_of::<$record>(),
            align_of::<$record>(),
            $fields
                .iter()
                .map(|(field, offset)| format!("{field}={offset}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    };
}

/// Prints the layout of every `#[repr(C)]` record the header declares.
#[test]
fn abi_layout_report() {
    report!("SpokeConnectSlice", SpokeConnectSlice, &[
        ("data", offset_of!(SpokeConnectSlice, data)),
        ("len", offset_of!(SpokeConnectSlice, len)),
    ]);
    report!("SpokeConnectBuffer", SpokeConnectBuffer, &[
        ("data", offset_of!(SpokeConnectBuffer, data)),
        ("len", offset_of!(SpokeConnectBuffer, len)),
    ]);
    report!(
        "SpokeConnectOptionalBuffer",
        SpokeConnectOptionalBuffer,
        &[
            ("present", offset_of!(SpokeConnectOptionalBuffer, present)),
            ("value", offset_of!(SpokeConnectOptionalBuffer, value)),
        ]
    );
    report!("SpokeConnectOptionalU64", SpokeConnectOptionalU64, &[
        ("present", offset_of!(SpokeConnectOptionalU64, present)),
        ("value", offset_of!(SpokeConnectOptionalU64, value)),
    ]);
    report!(
        "SpokeConnectForeignBuffer",
        SpokeConnectForeignBuffer,
        &[
            ("data", offset_of!(SpokeConnectForeignBuffer, data)),
            ("len", offset_of!(SpokeConnectForeignBuffer, len)),
            (
                "release_context",
                offset_of!(SpokeConnectForeignBuffer, release_context)
            ),
            ("release", offset_of!(SpokeConnectForeignBuffer, release)),
        ]
    );
    report!(
        "SpokeConnectForeignError",
        SpokeConnectForeignError,
        &[
            ("message", offset_of!(SpokeConnectForeignError, message)),
            ("code", offset_of!(SpokeConnectForeignError, code)),
            ("kind", offset_of!(SpokeConnectForeignError, kind)),
            ("wire_code", offset_of!(SpokeConnectForeignError, wire_code)),
        ]
    );
    report!("SpokeConnectError", SpokeConnectError, &[
        ("message", offset_of!(SpokeConnectError, message)),
        ("code", offset_of!(SpokeConnectError, code)),
        ("kind", offset_of!(SpokeConnectError, kind)),
        ("wire_code", offset_of!(SpokeConnectError, wire_code)),
        ("expected", offset_of!(SpokeConnectError, expected)),
        ("actual", offset_of!(SpokeConnectError, actual)),
    ]);
    report!("SpokeConnectPeerKey", SpokeConnectPeerKey, &[
        ("peer_id", offset_of!(SpokeConnectPeerKey, peer_id)),
        ("public_key", offset_of!(SpokeConnectPeerKey, public_key)),
    ]);
    report!(
        "SpokeConnectTransportTable",
        SpokeConnectTransportTable,
        &[
            ("send", offset_of!(SpokeConnectTransportTable, send)),
            ("recv", offset_of!(SpokeConnectTransportTable, recv)),
            ("close", offset_of!(SpokeConnectTransportTable, close)),
            ("destroy", offset_of!(SpokeConnectTransportTable, destroy)),
        ]
    );
    report!(
        "SpokeConnectPortsHandlerTable",
        SpokeConnectPortsHandlerTable,
        &[
            (
                "get_knowledge_entry",
                offset_of!(SpokeConnectPortsHandlerTable, get_knowledge_entry)
            ),
            (
                "put_knowledge_entry",
                offset_of!(SpokeConnectPortsHandlerTable, put_knowledge_entry)
            ),
            (
                "get_relation",
                offset_of!(SpokeConnectPortsHandlerTable, get_relation)
            ),
            (
                "put_relation",
                offset_of!(SpokeConnectPortsHandlerTable, put_relation)
            ),
            (
                "list_knowledge_entries",
                offset_of!(SpokeConnectPortsHandlerTable, list_knowledge_entries)
            ),
            (
                "list_timeline_events",
                offset_of!(SpokeConnectPortsHandlerTable, list_timeline_events)
            ),
            (
                "put_findings",
                offset_of!(SpokeConnectPortsHandlerTable, put_findings)
            ),
            (
                "list_rules",
                offset_of!(SpokeConnectPortsHandlerTable, list_rules)
            ),
            (
                "list_peer_host_capability_manifests",
                offset_of!(SpokeConnectPortsHandlerTable, list_peer_host_capability_manifests)
            ),
            ("project", offset_of!(SpokeConnectPortsHandlerTable, project)),
            ("compute", offset_of!(SpokeConnectPortsHandlerTable, compute)),
            (
                "list_fork_timeline_events",
                offset_of!(SpokeConnectPortsHandlerTable, list_fork_timeline_events)
            ),
            ("extract", offset_of!(SpokeConnectPortsHandlerTable, extract)),
            ("destroy", offset_of!(SpokeConnectPortsHandlerTable, destroy)),
        ]
    );
    report!(
        "SpokeConnectToolHandlerTable",
        SpokeConnectToolHandlerTable,
        &[
            ("handle", offset_of!(SpokeConnectToolHandlerTable, handle)),
            ("destroy", offset_of!(SpokeConnectToolHandlerTable, destroy)),
        ]
    );
}
