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
//! Non-callback members are reported a third way: as the Rust type of the
//! field itself. A same-size retype — `int64_t` to `uint64_t`, a dropped
//! `const` — leaves every size, alignment and offset unchanged, so only the
//! member type identifies it; the gate renders that Rust type back into the C
//! type the header must declare. The type is inferred from the field accessor
//! with no expected return type, so a coercion (`*mut u8` to `*const u8`)
//! cannot stand in for the field's own spelling, and the report stops unless
//! that inferred type is exactly the declared one.
//!
//! The report runs under `cargo test -p spoke-connect-capi --lib abi_layout --
//! --nocapture`, which the gate drives itself; it is a verification surface and
//! never part of the carrier's exported ABI.

use std::any::{type_name, TypeId};
use std::ffi::c_void;
use std::marker::PhantomData;
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

/// The type a field accessor returns, inferred with no expected type in the
/// return position (so a coercion like `*mut u8` to `*const u8` cannot stand
/// in for it), plus whether that type is exactly `$ty` — `TypeId` identity,
/// where a convertible type is a different id.
fn field_type_outcome<R, T: 'static, E: 'static>(
    _: fn(R) -> T,
    _: PhantomData<E>,
) -> (&'static str, bool) {
    (type_name::<T>(), TypeId::of::<T>() == TypeId::of::<E>())
}

/// Reports one non-callback member as `SPOKE_CONNECT_ABI_FIELD_TYPE
/// <record>.<field> <rust type>`. The reported type is the accessor's own
/// inferred type, and the identity check stops the report unless it is exactly
/// the declared one, so the line states the field's type — never a coercion
/// toward an expected one.
macro_rules! field_type {
    ($record:ty, $field:ident, $ty:ty) => {{
        let (reported, exact) = field_type_outcome(
            |record: $record| record.$field,
            PhantomData::<$ty>,
        );
        assert!(
            exact,
            "carrier field type drift at {}.{}: report declares {}, the field has {}",
            stringify!($record),
            stringify!($field),
            type_name::<$ty>(),
            reported
        );
        println!(
            "SPOKE_CONNECT_ABI_FIELD_TYPE {}.{} {}",
            stringify!($record),
            stringify!($field),
            reported
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
    report_field_types();
    constness_control();
}

/// Prints every non-callback record member the header declares, as the Rust
/// type the carrier's mirror actually has.
///
/// Callback members are deliberately absent: the callback axis already
/// compares their whole signature, which is strictly stronger than the member
/// type alone. What this adds is the member type the size/offset assertions
/// cannot see — an `int64_t` / `uint64_t` swap or a dropped `const` keeps every
/// offset and every size intact.
///
/// Called from `abi_layout_report` for the same one-writer reason as
/// `report_callbacks`.
fn report_field_types() {
    field_type!(SpokeConnectSlice, data, *const u8);
    field_type!(SpokeConnectSlice, len, usize);

    field_type!(SpokeConnectBuffer, data, *mut u8);
    field_type!(SpokeConnectBuffer, len, usize);

    field_type!(SpokeConnectOptionalBuffer, present, u8);
    field_type!(SpokeConnectOptionalBuffer, value, SpokeConnectBuffer);

    field_type!(SpokeConnectOptionalU64, present, u8);
    field_type!(SpokeConnectOptionalU64, value, u64);

    field_type!(SpokeConnectForeignBuffer, data, *const u8);
    field_type!(SpokeConnectForeignBuffer, len, usize);
    field_type!(SpokeConnectForeignBuffer, release_context, *mut c_void);

    field_type!(
        SpokeConnectForeignError,
        message,
        SpokeConnectForeignBuffer
    );
    field_type!(SpokeConnectForeignError, code, SpokeConnectForeignBuffer);
    field_type!(SpokeConnectForeignError, kind, SpokeConnectForeignBuffer);
    field_type!(
        SpokeConnectForeignError,
        wire_code,
        SpokeConnectForeignBuffer
    );

    field_type!(SpokeConnectError, message, SpokeConnectBuffer);
    field_type!(SpokeConnectError, code, SpokeConnectBuffer);
    field_type!(SpokeConnectError, kind, SpokeConnectBuffer);
    field_type!(SpokeConnectError, wire_code, SpokeConnectBuffer);
    field_type!(SpokeConnectError, expected, u64);
    field_type!(SpokeConnectError, actual, i64);

    field_type!(SpokeConnectPeerKey, peer_id, SpokeConnectSlice);
    field_type!(SpokeConnectPeerKey, public_key, SpokeConnectSlice);
}

/// The producer must report the field's own type: a `*mut u8` field is
/// reported as `*mut u8` — never the `*const u8` a coercing accessor could
/// silently satisfy — and the exact-identity check rejects that const spelling.
/// Called from `abi_layout_report` rather than declared as its own `#[test]`,
/// for the same one-writer reason as `report_callbacks`; it never reaches the
/// shipped library.
fn constness_control() {
    #[repr(C)]
    struct PointerConstnessProbe {
        mutable: *mut u8,
    }

    let (reported, exact) = field_type_outcome(
        |record: PointerConstnessProbe| record.mutable,
        PhantomData::<*mut u8>,
    );
    assert_eq!(reported, "*mut u8");
    assert!(exact);

    let (reported, exact) = field_type_outcome(
        |record: PointerConstnessProbe| record.mutable,
        PhantomData::<*const u8>,
    );
    assert_eq!(reported, "*mut u8");
    assert!(
        !exact,
        "a *mut u8 field must not satisfy the *const u8 report"
    );
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
