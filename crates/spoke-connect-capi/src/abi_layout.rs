//! Record-layout and callback-signature report for the C ABI gate.
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
//! It also reports every callback signature, because a callback table's
//! function pointers keep their size and their offsets when a parameter, a
//! return type or a calling convention changes — layout assertions alone cannot
//! see that. `type_name` renders the alias or the member type the carrier
//! actually has (nothing here restates a signature by hand), and the gate
//! renders that Rust type into the C signature it must equal. The member
//! reports additionally coerce a field accessor to an `fn` pointer, so a member
//! whose type stops matching the named one is a compile error rather than a
//! silently different report.
//!
//! The report runs under `cargo test -p spoke-connect-capi --lib abi_layout --
//! --nocapture`, which the gate drives itself; it is a verification surface and
//! never part of the carrier's exported ABI.

use std::any::type_name;
use std::ffi::c_void;
use std::mem::{align_of, offset_of, size_of};

use crate::remote_adapter::{
    SpokeConnectTransportCloseFn, SpokeConnectTransportDestroyFn, SpokeConnectTransportRecvFn,
    SpokeConnectTransportSendFn, SpokeConnectTransportTable,
};
use crate::responder::{
    SpokeConnectCallbackDestroyFn, SpokeConnectPeerKey, SpokeConnectPortsHandlerTable,
    SpokeConnectPortsListRulesFn, SpokeConnectPortsNoInputFn, SpokeConnectPortsRevisionFn,
    SpokeConnectPortsTextFn, SpokeConnectToolHandleFn, SpokeConnectToolHandlerTable,
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

/// Reports one callback typedef as `SPOKE_CONNECT_ABI_CALLBACK typedef <name>
/// <rust type>`. The name is the alias's own path (the header names its
/// typedef identically) and the type is the alias's real expansion, so a
/// changed parameter, return type or calling convention changes this line.
macro_rules! callback_typedef {
    ($alias:ty) => {
        println!(
            "SPOKE_CONNECT_ABI_CALLBACK typedef {} {}",
            stringify!($alias),
            type_name::<$alias>()
        );
    };
}

/// Reports one callback member as `SPOKE_CONNECT_ABI_CALLBACK member
/// <record>.<field> <rust type>`. The coerced closure is the typed probe: it
/// only compiles while the field's type is exactly `Option<$alias>`, so the
/// reported signature is the field's own — not a restatement of it.
macro_rules! callback_member {
    ($record:ty, $field:ident, $alias:ty) => {{
        let _: fn($record) -> Option<$alias> = |record| record.$field;
        println!(
            "SPOKE_CONNECT_ABI_CALLBACK member {}.{} {}",
            stringify!($record),
            stringify!($field),
            type_name::<$alias>()
        );
    }};
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
    report_callbacks();
}

/// Prints every callback typedef and every callback-table member the header
/// declares.
///
/// Called from `abi_layout_report` rather than declared as its own `#[test]`:
/// the gate parses the whole report from one `cargo test -- --nocapture` run,
/// and a second test would let the harness write its own status line from
/// another thread while this one is mid-report, gluing a report line onto it.
/// One test means one writer, so each report line reaches the gate intact.
fn report_callbacks() {
    callback_typedef!(SpokeConnectTransportSendFn);
    callback_typedef!(SpokeConnectTransportRecvFn);
    callback_typedef!(SpokeConnectTransportCloseFn);
    callback_typedef!(SpokeConnectTransportDestroyFn);
    callback_typedef!(SpokeConnectCallbackDestroyFn);
    callback_typedef!(SpokeConnectPortsTextFn);
    callback_typedef!(SpokeConnectPortsRevisionFn);
    callback_typedef!(SpokeConnectPortsListRulesFn);
    callback_typedef!(SpokeConnectPortsNoInputFn);
    callback_typedef!(SpokeConnectToolHandleFn);

    callback_member!(SpokeConnectTransportTable, send, SpokeConnectTransportSendFn);
    callback_member!(SpokeConnectTransportTable, recv, SpokeConnectTransportRecvFn);
    callback_member!(SpokeConnectTransportTable, close, SpokeConnectTransportCloseFn);
    callback_member!(
        SpokeConnectTransportTable,
        destroy,
        SpokeConnectTransportDestroyFn
    );

    // The foreign buffer's `release` is the one callback member the header
    // writes inline instead of naming a typedef for.
    callback_member!(
        SpokeConnectForeignBuffer,
        release,
        unsafe extern "C" fn(*mut c_void, *const u8, usize)
    );

    callback_member!(
        SpokeConnectPortsHandlerTable,
        get_knowledge_entry,
        SpokeConnectPortsTextFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        put_knowledge_entry,
        SpokeConnectPortsRevisionFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        get_relation,
        SpokeConnectPortsTextFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        put_relation,
        SpokeConnectPortsRevisionFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        list_knowledge_entries,
        SpokeConnectPortsTextFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        list_timeline_events,
        SpokeConnectPortsTextFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        put_findings,
        SpokeConnectPortsTextFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        list_rules,
        SpokeConnectPortsListRulesFn
    );
    callback_member!(
        SpokeConnectPortsHandlerTable,
        list_peer_host_capability_manifests,
        SpokeConnectPortsNoInputFn
    );
    callback_member!(SpokeConnectPortsHandlerTable, project, SpokeConnectPortsTextFn);
    callback_member!(SpokeConnectPortsHandlerTable, compute, SpokeConnectPortsTextFn);
    callback_member!(
        SpokeConnectPortsHandlerTable,
        list_fork_timeline_events,
        SpokeConnectPortsTextFn
    );
    callback_member!(SpokeConnectPortsHandlerTable, extract, SpokeConnectPortsTextFn);
    callback_member!(
        SpokeConnectPortsHandlerTable,
        destroy,
        SpokeConnectCallbackDestroyFn
    );

    callback_member!(SpokeConnectToolHandlerTable, handle, SpokeConnectToolHandleFn);
    callback_member!(
        SpokeConnectToolHandlerTable,
        destroy,
        SpokeConnectCallbackDestroyFn
    );
}
