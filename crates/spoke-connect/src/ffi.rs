//! uniffi-exported sync-core facade, behind the non-default `ffi` feature.
//!
//! This module is the FFI contract for foreign-language bindings (Swift
//! first — see the crate README "Binding facade" section). It re-exposes the
//! pure session core ([`crate::core`]) with a uniffi-compatible surface:
//! every exported function and object maps 1:1 onto a core rule, and the
//! core itself stays untouched, dependency-pure, and uniffi-free.
//!
//! Boundary conventions:
//! - Keys cross as raw `Vec<u8>` — uniffi has no fixed-size array type — and
//!   are validated to exactly 32 bytes inside the wrapper.
//! - Peer ids cross as `String`; the host manifest and the hello envelope
//!   cross as JSON strings, deserialized with `serde_json` inside Rust, so
//!   no generated schema types appear on the FFI surface.
//! - Errors map to thin FFI-facing enums that mirror
//!   [`crate::core::CoreError`] / [`crate::core::CoreInvokeError`]
//!   variant-for-variant; the core enums are unchanged.
//!
//! ## Envelope-auth deferral (no new FFI APIs)
//!
//! The v2 envelope-authentication helpers ([`crate::core::envelope_auth`]:
//! `authenticate_*` / `verify_*` for `ConnectSession`, `ConnectInvokeRequest`,
//! `ConnectInvokeResponse`) are deliberately **not** exposed through this
//! facade. The frozen envelope-auth contract §9 locks "no new FFI APIs":
//! bindings keep calling the encapsulated RemoteAdapter / connect-client
//! surfaces, which attach and verify authenticators internally — verify
//! helpers are never host-callable (encapsulation hard rule), so widening the
//! FFI surface would add binding-parity surface for no consumer benefit. The
//! parity gate for the new auth surface is the TS↔Rust session core
//! (`crates/spoke-connect/src/core/envelope_auth.rs` ↔
//! `packages/spoke-connect-ts/src/core/envelope-auth.ts`): canonical bytes,
//! algorithm ids, and verify outcomes. Binding golden-parity smokes covering
//! hello stay green and are not extended to envelope auth.

use std::any::Any;
use std::sync::{Arc, Mutex};
#[cfg(feature = "ffi")]
use std::sync::OnceLock;

#[cfg(feature = "ffi")]
static FFI_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Process-wide tokio runtime for the FFI surface (AR-1: multi-thread).
#[cfg(feature = "ffi")]
pub(crate) fn ffi_runtime() -> &'static tokio::runtime::Runtime {
    FFI_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("cdylib tokio runtime initializes once")
    })
}


/// Extract a human-readable message from a `catch_unwind` payload.
fn panic_payload_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message.to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

// ── Foreign-callback `Transport` over the async seam (AR-2 / AR-3) ───────
//
// The `RemoteAdapter`'s `Transport` seam is async (`send`/`recv`/`close`,
// `Send + Sync`). Over FFI the binding implements a *synchronous* callback
// `Transport` — a binding's `recv` blocks until an envelope arrives or the
// connection closes (the message-transport model). [`ForeignCallbackTransport`]
// bridges the two: each foreign callback is run through the shared runtime's
// `spawn_blocking` pool so a blocking `recv` never monopolizes an async
// worker (AR-2), and close ordering is fixed end-to-end (AR-3) — a foreign
// `close` makes a pending/next `recv` fail fast with [`TransportError::Closed`].
#[cfg(feature = "remote-adapter")]
mod foreign_transport {
    use async_trait::async_trait;
    use std::future::Future;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Arc;

    use crate::remote::transport;
    use crate::remote::transport::Transport as RemoteAsyncTransport;

    use super::{ffi_runtime, panic_payload_message};

    fn ffi_block_on_transport<F, T>(future: F) -> Result<T, TransportError>
    where
        F: Future<Output = Result<T, transport::TransportError>>,
    {
        match catch_unwind(AssertUnwindSafe(|| ffi_runtime().block_on(future))) {
            Ok(result) => result.map_err(Into::into),
            Err(payload) => Err(TransportError::Io(format!(
                "internal panic: {}",
                panic_payload_message(payload)
            ))),
        }
    }

    /// FFI-facing mirror of [`transport::TransportError`] — the
    /// callback `Transport`'s own error vocabulary. 1:1 with the remote
    /// error; the bridge maps both directions.
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
    pub enum TransportError {
        /// The transport is closed. A pending `recv` must fail fast on
        /// connection loss so the adapter can fail its in-flight invokes.
        #[error("transport is closed")]
        Closed,
        /// Transport-level I/O failure.
        #[error("transport I/O error: {0}")]
        Io(String),
    }

    impl From<transport::TransportError> for TransportError {
        fn from(error: transport::TransportError) -> Self {
            match error {
                transport::TransportError::Closed => TransportError::Closed,
                transport::TransportError::Io(message) => TransportError::Io(message),
            }
        }
    }

    impl From<TransportError> for transport::TransportError {
        fn from(error: TransportError) -> Self {
            match error {
                TransportError::Closed => transport::TransportError::Closed,
                TransportError::Io(message) => transport::TransportError::Io(message),
            }
        }
    }

    /// Message-oriented transport implemented by the foreign binding.
    ///
    /// Mirrors the async [`transport::Transport`] seam 1:1 over the
    /// FFI boundary (frozen contract §2.1): `send` accepts exactly one
    /// envelope's bytes, `recv` returns the next inbound envelope and fails
    /// fast on close, `close` is idempotent resource release.
    #[uniffi::export(callback_interface)]
    pub trait Transport: Send + Sync {
        /// Send one envelope. Resolves when the transport has accepted the
        /// bytes.
        fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError>;
        /// Receive the next inbound envelope. Errors when the transport
        /// closes.
        fn recv(&self) -> Result<Vec<u8>, TransportError>;
        /// Release resources. Idempotent.
        fn close(&self) -> Result<(), TransportError>;
    }

    /// Async bridge from a synchronous foreign-callback [`Transport`] to the
    /// async [`transport::Transport`] seam (AR-2).
    ///
    /// Each callback call is executed via `runtime.spawn_blocking(...)` on
    /// the shared FFI runtime, so a blocking foreign `recv` runs on the
    /// runtime's blocking thread pool — never on an async worker. The
    /// `RemoteAdapter`'s receive loop stays a normal async task; only the
    /// foreign-callback invocation is offloaded.
    #[derive(Clone)]
    pub struct ForeignCallbackTransport {
        inner: Arc<dyn Transport>,
    }

    impl ForeignCallbackTransport {
        /// Wrap a foreign-callback [`Transport`] into an async-capable
        /// transport.
        #[must_use]
        pub fn new(inner: Arc<dyn Transport>) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl transport::Transport for ForeignCallbackTransport {
        async fn send(&self, envelope: &[u8]) -> Result<(), transport::TransportError> {
            let inner = Arc::clone(&self.inner);
            let envelope = envelope.to_vec();
            ffi_runtime()
                .spawn_blocking(move || inner.send(envelope))
                .await
                .map_err(|join| {
                    transport::TransportError::Io(format!("send task failed: {join}"))
                })?
                .map_err(Into::into)
        }

        async fn recv(&self) -> Result<Vec<u8>, transport::TransportError> {
            let inner = Arc::clone(&self.inner);
            ffi_runtime()
                .spawn_blocking(move || inner.recv())
                .await
                .map_err(|join| {
                    transport::TransportError::Io(format!("recv task failed: {join}"))
                })?
                .map_err(Into::into)
        }

        async fn close(&self) -> Result<(), transport::TransportError> {
            let inner = Arc::clone(&self.inner);
            ffi_runtime()
                .spawn_blocking(move || inner.close())
                .await
                .map_err(|join| {
                    transport::TransportError::Io(format!("close task failed: {join}"))
                })?
                .map_err(Into::into)
        }
    }

    /// One end of an in-memory loopback connection, exposed over FFI (AR-7)
    /// so a binding can exercise the callback `Transport` surface without a
    /// real network carrier. `send` delivers to the peer's `recv`; `close`
    /// closes the whole connection (both directions). Each method
    /// `block_on`s the shared runtime — the same synchronous block-on-async
    /// surface a binding uses (AR-1 / AR-6).
    #[derive(uniffi::Object)]
    pub struct LoopbackTransport {
        inner: transport::LoopbackTransport,
    }

    #[uniffi::export]
    impl LoopbackTransport {
        /// Send one envelope; delivered to the peer end's `recv`.
        pub fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError> {
            ffi_block_on_transport(self.inner.send(&envelope))
        }

        /// Receive the next inbound envelope. Errors when the connection
        /// closes.
        pub fn recv(&self) -> Result<Vec<u8>, TransportError> {
            ffi_block_on_transport(self.inner.recv())
        }

        /// Close the whole connection (both directions). Idempotent.
        pub fn close(&self) -> Result<(), TransportError> {
            ffi_block_on_transport(self.inner.close())
        }
    }

    impl LoopbackTransport {
        fn from_remote(inner: transport::LoopbackTransport) -> Arc<Self> {
            Arc::new(Self { inner })
        }

        /// Clone the async loopback end wrapped by this FFI object.
        pub(crate) fn clone_async_inner(&self) -> transport::LoopbackTransport {
            self.inner.clone()
        }
    }

    /// Back-to-back loopback transport pair — `client` and `server` ends of
    /// the same in-memory connection (mirror of
    /// [`transport::loopback_transport_pair`]).
    #[derive(uniffi::Object)]
    pub struct LoopbackTransportPair {
        client: Arc<LoopbackTransport>,
        server: Arc<LoopbackTransport>,
    }

    #[uniffi::export]
    impl LoopbackTransportPair {
        /// The client end of the connection.
        pub fn client(&self) -> Arc<LoopbackTransport> {
            Arc::clone(&self.client)
        }

        /// The server end of the connection.
        pub fn server(&self) -> Arc<LoopbackTransport> {
            Arc::clone(&self.server)
        }
    }

    /// Create a back-to-back loopback transport pair (client + server ends).
    #[uniffi::export]
    pub fn loopback_transport_pair() -> LoopbackTransportPair {
        let pair = transport::loopback_transport_pair();
        LoopbackTransportPair {
            client: LoopbackTransport::from_remote(pair.client),
            server: LoopbackTransport::from_remote(pair.server),
        }
    }
}

#[cfg(feature = "remote-adapter")]
pub use foreign_transport::{
    loopback_transport_pair, ForeignCallbackTransport, LoopbackTransport, LoopbackTransportPair,
    Transport, TransportError,
};

// ── Sync `RemoteAdapterFFI` over the async adapter (AR-4 / AR-5 / AR-6) ───
#[cfg(feature = "remote-adapter")]
mod remote_adapter_ffi {
    use std::any::Any;
    #[cfg(test)]
    use std::cell::Cell;
    use std::future::Future;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Arc;

    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::Value;
    use spoke_operations::{
        FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort,
        ScopeQueryPort, SpokeReject, SpokeResult,
    };
    use spoke_schemas::{Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Scope};

    use crate::remote::{
        connect_remote_adapter, RemoteAdapter, RemoteAdapterError, RemoteAdapterOptions,
        RemoteIdentity,
    };

    use super::foreign_transport::ForeignCallbackTransport;
    use super::foreign_transport::Transport as FfiTransport;
    use super::{ffi_runtime, panic_payload_message};

    #[cfg(test)]
    thread_local! {
        static INJECT_PANIC_ON_NEXT_FFI_BLOCK_ON: Cell<bool> = Cell::new(false);
    }

    fn map_block_on_panic(payload: Box<dyn Any + Send>) -> FfiError {
        FfiError::Rejected {
            code: "INTERNAL_ERROR".into(),
            message: panic_payload_message(payload),
            kind: Some("panic".into()),
            wire_code: None,
        }
    }

    #[cfg(not(test))]
    pub(super) fn ffi_block_on<F, T>(future: F) -> Result<T, FfiError>
    where
        F: Future<Output = T>,
    {
        match catch_unwind(AssertUnwindSafe(|| ffi_runtime().block_on(future))) {
            Ok(value) => Ok(value),
            Err(payload) => Err(map_block_on_panic(payload)),
        }
    }

    #[cfg(test)]
    pub(super) fn ffi_block_on<F, T>(future: F) -> Result<T, FfiError>
    where
        F: Future<Output = T>,
    {
        let future = async {
            if INJECT_PANIC_ON_NEXT_FFI_BLOCK_ON.with(|flag| {
                let should_panic = flag.get();
                if should_panic {
                    flag.set(false);
                }
                should_panic
            }) {
                panic!("injected ffi block_on panic");
            }
            future.await
        };
        match catch_unwind(AssertUnwindSafe(|| ffi_runtime().block_on(future))) {
            Ok(value) => Ok(value),
            Err(payload) => Err(map_block_on_panic(payload)),
        }
    }

    pub(super) fn ffi_block_on_void<F>(future: F)
    where
        F: Future<Output = ()>,
    {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            ffi_runtime().block_on(future);
        })) {
            eprintln!(
                "ffi: swallowed panic during close: {}",
                panic_payload_message(payload)
            );
        }
    }

    #[cfg(test)]
    pub(super) fn inject_panic_on_next_block_on_for_test() {
        INJECT_PANIC_ON_NEXT_FFI_BLOCK_ON.with(|flag| flag.set(true));
    }


    /// FFI error surface — 1:1 with frozen-contract D7 (AR-5).
    ///
    /// - [`FfiError::Dial`] — constructor / dial failures before an adapter
    ///   exists (`config` / `handshake` / `timeout`).
    /// - [`FfiError::Rejected`] — invoke-path `SpokeResult::Reject` passthrough:
    ///   application codes preserved; `INTERNAL_ERROR` rows carry `kind`;
    ///   dispatch deny and unknown wire codes carry `wire_code`.
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
    pub enum FfiError {
        #[error("dial failed ({kind}): {message}")]
        Dial { kind: String, message: String },
        #[error("rejected ({code}): {message}")]
        Rejected {
            code: String,
            message: String,
            kind: Option<String>,
            wire_code: Option<String>,
        },
    }

    /// Map [`RemoteAdapterError`] (dial-only) to [`FfiError::Dial`] (D7 last row).
    fn map_dial_error(error: RemoteAdapterError) -> FfiError {
        match error {
            RemoteAdapterError::Config(message) => FfiError::Dial {
                kind: "config".into(),
                message,
            },
            RemoteAdapterError::Handshake(message) => FfiError::Dial {
                kind: "handshake".into(),
                message,
            },
            RemoteAdapterError::Timeout(message) => FfiError::Dial {
                kind: "timeout".into(),
                message,
            },
        }
    }

    fn detail_string(details: &Option<serde_json::Map<String, Value>>, key: &str) -> Option<String> {
        details
            .as_ref()
            .and_then(|details| details.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// Map invoke-path `SpokeReject` to [`FfiError::Rejected`] — faithful D7
    /// passthrough (AR-5): no invented, merged, or dropped error classes.
    pub(super) fn map_spoke_reject(reject: SpokeReject) -> FfiError {
        FfiError::Rejected {
            code: reject.code.as_str().to_string(),
            message: reject.message,
            kind: detail_string(&reject.details, "kind"),
            wire_code: detail_string(&reject.details, "wire_code"),
        }
    }

    pub(super) fn map_spoke_result<T: Serialize>(result: SpokeResult<T>) -> Result<String, FfiError> {
        match result {
            SpokeResult::Ok(value) => serde_json::to_string(&value).map_err(|error| {
                FfiError::Rejected {
                    code: "INTERNAL_ERROR".into(),
                    message: format!("response serialize failed: {error}"),
                    kind: Some("transport".into()),
                    wire_code: None,
                }
            }),
            SpokeResult::Reject(reject) => Err(map_spoke_reject(reject)),
        }
    }

    pub(super) fn parse_json_field<T: DeserializeOwned>(json: &str, what: &str) -> Result<T, FfiError> {
        serde_json::from_str(json).map_err(|error| FfiError::Rejected {
            code: "INVALID_INPUT".into(),
            message: format!("invalid {what} JSON: {error}"),
            kind: None,
            wire_code: None,
        })
    }

    fn ed25519_seed(bytes: Vec<u8>) -> Result<[u8; 32], FfiError> {
        bytes.try_into().map_err(|_| FfiError::Dial {
            kind: "config".into(),
            message: "local seed must be exactly 32 bytes".into(),
        })
    }

    fn ed25519_pubkey(bytes: Vec<u8>) -> Result<[u8; 32], FfiError> {
        bytes.try_into().map_err(|_| FfiError::Dial {
            kind: "config".into(),
            message: "remote public key must be exactly 32 bytes".into(),
        })
    }

    #[derive(uniffi::Object)]
    pub struct RemoteAdapterFFI {
        inner: Arc<RemoteAdapter>,
    }

    #[uniffi::export]
    impl RemoteAdapterFFI {
        pub fn state(&self) -> String {
            self.inner.state().as_str().to_string()
        }

        pub fn session_id(&self) -> Option<String> {
            self.inner.session_id()
        }

        pub fn remote_peer_id(&self) -> Option<String> {
            self.inner.remote_peer_id()
        }

        pub fn remote_manifest(&self) -> Option<String> {
            self.inner.remote_manifest().and_then(|manifest| {
                serde_json::to_string(&manifest).ok()
            })
        }

        pub fn get_host_capability_manifest(&self) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.inner.get_host_capability_manifest())?)
        }

        pub fn get_knowledge_entry(&self, entry_id: String) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.inner.get_knowledge_entry(&entry_id))?)
        }

        pub fn put_knowledge_entry(
            &self,
            entry_json: String,
            expected_base_revision: Option<u64>,
        ) -> Result<String, FfiError> {
            let entry: KnowledgeEntry = parse_json_field(&entry_json, "knowledge entry")?;
            map_spoke_result(ffi_block_on(
                self.inner
                    .put_knowledge_entry(entry, expected_base_revision),
            )?)
        }

        pub fn get_relation(&self, relation_id: String) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.inner.get_relation(&relation_id))?)
        }

        pub fn put_relation(
            &self,
            relation_json: String,
            expected_base_revision: Option<u64>,
        ) -> Result<String, FfiError> {
            let relation: Relation = parse_json_field(&relation_json, "relation")?;
            map_spoke_result(ffi_block_on(
                self.inner.put_relation(relation, expected_base_revision),
            )?)
        }

        pub fn list_knowledge_entries(&self, scope_json: String) -> Result<String, FfiError> {
            let scope: Scope = parse_json_field(&scope_json, "scope")?;
            map_spoke_result(ffi_block_on(self.inner.list_knowledge_entries(&scope))?)
        }

        pub fn list_timeline_events(&self, scope_json: String) -> Result<String, FfiError> {
            let scope: Scope = parse_json_field(&scope_json, "scope")?;
            map_spoke_result(ffi_block_on(self.inner.list_timeline_events(&scope))?)
        }

        pub fn put_findings(&self, findings_json: String) -> Result<String, FfiError> {
            let findings: Vec<Finding> = parse_json_field(&findings_json, "findings")?;
            map_spoke_result(ffi_block_on(self.inner.put_findings(findings))?)
        }

        pub fn list_rules(&self, rule_refs: Vec<String>) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.inner.list_rules(&rule_refs))?)
        }

        pub fn list_peer_host_capability_manifests(&self) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.inner.list_peer_host_capability_manifests())?)
        }

        pub fn close(&self) {
            ffi_block_on_void(async {
                self.inner.close();
            });
        }
    }

    impl RemoteAdapterFFI {
        pub(crate) fn inner_adapter(&self) -> Arc<RemoteAdapter> {
            Arc::clone(&self.inner)
        }
    }

    #[uniffi::export]
    pub fn connect_remote_adapter_ffi(
        transport: Box<dyn FfiTransport>,
        local_seed: Vec<u8>,
        local_manifest_json: String,
        remote_pubkey: Vec<u8>,
        allowlist: Vec<String>,
        invoke_timeout_ms: Option<u64>,
    ) -> Result<Arc<RemoteAdapterFFI>, FfiError> {
        let local_manifest: HostCapabilityManifest = serde_json::from_str(&local_manifest_json)
            .map_err(|error| FfiError::Dial {
                kind: "config".into(),
                message: format!("invalid local host manifest JSON: {error}"),
            })?;
        let adapter = ffi_block_on(connect_remote_adapter(RemoteAdapterOptions {
                transport: Arc::new(ForeignCallbackTransport::new(Arc::from(transport))),
                local_identity: RemoteIdentity {
                    seed: ed25519_seed(local_seed)?,
                },
                local_manifest,
                remote_pubkey: ed25519_pubkey(remote_pubkey)?,
                allowlist,
                invoke_timeout_ms,
                capability_token: None,
            }))?
            .map_err(map_dial_error)?;
        Ok(Arc::new(RemoteAdapterFFI { inner: adapter }))
    }


    #[cfg(test)]
    mod ffi_error_mapping_unit_tests {
        use serde_json::json;
        use spoke_operations::{SpokeReject, SpokeRejectCode};

        use super::{map_dial_error, map_spoke_reject, FfiError};
        use crate::remote::RemoteAdapterError;

        #[test]
        fn maps_dial_errors_to_ffi_dial_kinds() {
            for (error, kind) in [
                (
                    RemoteAdapterError::Config("bad config".into()),
                    "config",
                ),
                (
                    RemoteAdapterError::Handshake("bad hello".into()),
                    "handshake",
                ),
                (
                    RemoteAdapterError::Timeout("connect: hello timed out".into()),
                    "timeout",
                ),
            ] {
                let ffi = map_dial_error(error);
                assert!(matches!(ffi, FfiError::Dial { kind: k, .. } if k == kind));
            }
        }

        #[test]
        fn maps_internal_error_kinds_verbatim() {
            for kind in [
                "transport",
                "session_closed",
                "timeout",
                "correlation_mismatch",
                "sequence_exhausted",
                "envelope_auth_missing",
                "envelope_auth_invalid",
                "envelope_auth_session_unbound",
                "panic",
            ] {
                let reject = SpokeReject {
                    code: SpokeRejectCode::InternalError,
                    message: format!("{kind} failure"),
                    details: Some(json!({ "kind": kind }).as_object().unwrap().clone()),
                };
                let ffi = map_spoke_reject(reject);
                assert!(matches!(
                    ffi,
                    FfiError::Rejected {
                        code,
                        kind: Some(k),
                        wire_code: None,
                        ..
                    } if code == "INTERNAL_ERROR" && k == kind
                ));
            }
        }

        #[test]
        fn maps_application_reject_without_kind_or_wire_code() {
            let reject = SpokeReject {
                code: SpokeRejectCode::RevisionConflict,
                message: "revision mismatch".into(),
                details: None,
            };
            let ffi = map_spoke_reject(reject);
            assert!(matches!(
                ffi,
                FfiError::Rejected {
                    code,
                    kind: None,
                    wire_code: None,
                    ..
                } if code == "REVISION_CONFLICT"
            ));
        }

        #[test]
        fn maps_dispatch_deny_with_wire_code() {
            let reject = SpokeReject {
                code: SpokeRejectCode::CapabilityPortMissing,
                message: "op denied".into(),
                details: Some(json!({ "wire_code": "op_unsupported" }).as_object().unwrap().clone()),
            };
            let ffi = map_spoke_reject(reject);
            assert!(matches!(
                ffi,
                FfiError::Rejected {
                    code,
                    kind: None,
                    wire_code: Some(wire),
                    ..
                } if code == "CAPABILITY_PORT_MISSING" && wire == "op_unsupported"
            ));
        }

        #[test]
        fn maps_unknown_wire_code_to_invalid_input() {
            let reject = SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                message: "unknown host code".into(),
                details: Some(json!({ "wire_code": "totally_unknown" }).as_object().unwrap().clone()),
            };
            let ffi = map_spoke_reject(reject);
            assert!(matches!(
                ffi,
                FfiError::Rejected {
                    code,
                    wire_code: Some(wire),
                    ..
                } if code == "INVALID_INPUT" && wire == "totally_unknown"
            ));
        }
    }

    #[cfg(test)]
    impl RemoteAdapterFFI {
        pub(crate) fn from_adapter(adapter: Arc<RemoteAdapter>) -> Arc<Self> {
            Arc::new(Self { inner: adapter })
        }
    }
}

// ── Sync `MultiPeerRouterFFI` over the async router (AR-6 / D11) ─────────
#[cfg(feature = "remote-adapter")]
mod multi_peer_router_ffi {
    use std::sync::Arc;

    use spoke_operations::{
        FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort,
        ScopeQueryPort,
    };
    use spoke_schemas::{Finding, KnowledgeEntry, Relation, Scope};

    use crate::remote::{
        connect_multi_peer_router, MultiPeerRouter, MultiPeerRouterError,
        MultiPeerRouterOptions,
    };

    use super::remote_adapter_ffi::{
        ffi_block_on, map_spoke_result, parse_json_field, FfiError, RemoteAdapterFFI,
    };

    fn map_register_error(error: MultiPeerRouterError) -> FfiError {
        FfiError::Rejected {
            code: "INVALID_INPUT".into(),
            message: error.to_string(),
            kind: None,
            wire_code: None,
        }
    }

    #[derive(uniffi::Object)]
    pub struct MultiPeerRouterFFI {
        router: MultiPeerRouter,
    }

    #[uniffi::export]
    impl MultiPeerRouterFFI {
        pub fn register_peer(&self, adapter: Arc<RemoteAdapterFFI>) -> Result<String, FfiError> {
            self.router
                .register_peer(adapter.inner_adapter())
                .map_err(map_register_error)
        }

        pub fn unregister_peer(&self, peer_id: String) {
            self.router.unregister_peer(&peer_id);
        }

        pub fn list_peers(&self) -> Vec<String> {
            self.router.list_peers()
        }

        pub fn get_host_capability_manifest(&self) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.router.get_host_capability_manifest())?)
        }

        pub fn get_knowledge_entry(&self, entry_id: String) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.router.get_knowledge_entry(&entry_id))?)
        }

        pub fn put_knowledge_entry(
            &self,
            entry_json: String,
            expected_base_revision: Option<u64>,
        ) -> Result<String, FfiError> {
            let entry: KnowledgeEntry = parse_json_field(&entry_json, "knowledge entry")?;
            map_spoke_result(ffi_block_on(
                self.router
                    .put_knowledge_entry(entry, expected_base_revision),
            )?)
        }

        pub fn get_relation(&self, relation_id: String) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.router.get_relation(&relation_id))?)
        }

        pub fn put_relation(
            &self,
            relation_json: String,
            expected_base_revision: Option<u64>,
        ) -> Result<String, FfiError> {
            let relation: Relation = parse_json_field(&relation_json, "relation")?;
            map_spoke_result(ffi_block_on(
                self.router.put_relation(relation, expected_base_revision),
            )?)
        }

        pub fn list_knowledge_entries(&self, scope_json: String) -> Result<String, FfiError> {
            let scope: Scope = parse_json_field(&scope_json, "scope")?;
            map_spoke_result(ffi_block_on(self.router.list_knowledge_entries(&scope))?)
        }

        pub fn list_timeline_events(&self, scope_json: String) -> Result<String, FfiError> {
            let scope: Scope = parse_json_field(&scope_json, "scope")?;
            map_spoke_result(ffi_block_on(self.router.list_timeline_events(&scope))?)
        }

        pub fn put_findings(&self, findings_json: String) -> Result<String, FfiError> {
            let findings: Vec<Finding> = parse_json_field(&findings_json, "findings")?;
            map_spoke_result(ffi_block_on(self.router.put_findings(findings))?)
        }

        pub fn list_rules(&self, rule_refs: Vec<String>) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.router.list_rules(&rule_refs))?)
        }

        pub fn list_peer_host_capability_manifests(&self) -> Result<String, FfiError> {
            map_spoke_result(ffi_block_on(self.router.list_peer_host_capability_manifests())?)
        }
    }

    #[uniffi::export]
    pub fn new_multi_peer_router_ffi() -> Arc<MultiPeerRouterFFI> {
        Arc::new(MultiPeerRouterFFI {
            router: connect_multi_peer_router(MultiPeerRouterOptions::default()),
        })
    }
}


// ── Loopback smoke host (binding smokes) ─────────────────────────────────
#[cfg(feature = "ffi-smoke-host")]
mod loopback_smoke_host {
    use std::sync::Arc;

    use spoke_fixture_toy_world::ToyWorldAdapter;

    use crate::core::derive_peer_id_from_ed25519_pubkey;
    use crate::test_support::loopback_oracle::{
        manifest, pubkey_client, seed_host, start_loopback_host, LoopbackHost,
        LoopbackHostOptions,
    };

    use super::foreign_transport::LoopbackTransport;

    /// Reference loopback host for binding smokes (ToyWorld fixtures, fixed
    /// test seeds). Serves the server end of a [`loopback_transport_pair`].
    #[derive(uniffi::Object)]
    pub struct LoopbackSmokeHost {
        host: LoopbackHost,
    }

    #[uniffi::export]
    impl LoopbackSmokeHost {
        /// Fixed loopback session id (`test-session-loopback-0001`).
        pub fn session_id(&self) -> String {
            self.host.session_id().to_string()
        }

        /// Close the connection (fails the client's pending recv / invokes).
        pub fn close(&self) {
            super::remote_adapter_ffi::ffi_block_on_void(async {
                self.host.close();
            });
        }
    }

    fn start_loopback_smoke_host_inner(
        server: Arc<LoopbackTransport>,
        host_seed: [u8; 32],
        host_manifest: spoke_schemas::HostCapabilityManifest,
        adapter: Arc<ToyWorldAdapter>,
    ) -> Result<Arc<LoopbackSmokeHost>, super::remote_adapter_ffi::FfiError> {
        let transport = Arc::new(server.clone_async_inner());
        let host = super::remote_adapter_ffi::ffi_block_on(start_loopback_host(LoopbackHostOptions {
            transport,
            host_seed,
            host_manifest,
            allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
            adapter,
            delay: Box::new(|_| 0),
            response_override: None,
            session_peer_ids: None,
        }))?;
        Ok(Arc::new(LoopbackSmokeHost { host }))
    }

    /// Start the reference loopback smoke host on the server end of a
    /// loopback pair. Uses the same seeds, manifests, and ToyWorld adapter
    /// as the Rust FFI parity tests.
    #[uniffi::export]
    pub fn start_loopback_smoke_host(
        server: Arc<LoopbackTransport>,
    ) -> Arc<LoopbackSmokeHost> {
        // UniFFI export has no error slot; re-panic with the mapped FfiError message.
        start_loopback_smoke_host_inner(
            server,
            seed_host(),
            manifest("test-host", &["spoke-baseline"]),
            Arc::new(ToyWorldAdapter::with_committed_fixtures()),
        )
        .unwrap_or_else(|error| panic!("loopback smoke host start: {error}"))
    }

    /// Parametric loopback smoke host for multi-peer routing smokes: fixed
    /// client allowlist (`seed_client`), caller-supplied host seed + manifest.
    /// Uses `ToyWorldAdapter::default()` (same as Rust `multi_peer_router_ffi`
    /// loopback proofs). `ffi-smoke-host` only — not in production cdylib.
    #[uniffi::export]
    pub fn start_loopback_smoke_host_variant(
        server: Arc<LoopbackTransport>,
        host_seed: Vec<u8>,
        host_manifest_json: String,
    ) -> Result<Arc<LoopbackSmokeHost>, super::remote_adapter_ffi::FfiError> {
        let host_seed: [u8; 32] = host_seed.try_into().map_err(|_| {
            super::remote_adapter_ffi::FfiError::Dial {
                kind: "config".into(),
                message: "host seed must be exactly 32 bytes".into(),
            }
        })?;
        let host_manifest: spoke_schemas::HostCapabilityManifest =
            serde_json::from_str(&host_manifest_json).map_err(|error| {
                super::remote_adapter_ffi::FfiError::Dial {
                    kind: "config".into(),
                    message: format!("invalid host manifest JSON: {error}"),
                }
            })?;
        start_loopback_smoke_host_inner(
            server,
            host_seed,
            host_manifest,
            Arc::new(ToyWorldAdapter::default()),
        )
    }
}

#[cfg(feature = "remote-adapter")]
pub use remote_adapter_ffi::{connect_remote_adapter_ffi, FfiError, RemoteAdapterFFI};
#[cfg(feature = "remote-adapter")]
pub use multi_peer_router_ffi::{new_multi_peer_router_ffi, MultiPeerRouterFFI};
#[cfg(feature = "ffi-smoke-host")]
pub use loopback_smoke_host::{start_loopback_smoke_host, start_loopback_smoke_host_variant, LoopbackSmokeHost};

use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::ConnectHello;

use crate::core::{CoreError as CoreErrorImpl, CoreInvokeError as CoreInvokeErrorImpl};

/// FFI-facing mirror of [`crate::core::CoreError`] (hello-gate / identity
/// failures). Mapped 1:1 in [`From<CoreErrorImpl>`].
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// The hello signature did not verify against the peer's public key (or
    /// the signature is not valid base64url / not 64 bytes).
    #[error("hello signature invalid")]
    InvalidHelloSignature,
    /// The `(peer_id, nonce)` pair was already accepted.
    #[error("hello nonce replayed")]
    NonceReplay,
    /// Handshake-level failure (protocol version, peer id binding, …).
    #[error("handshake failed: {reason}")]
    HandshakeFailed { reason: String },
    /// The hello nonce does not satisfy the wire constraints (minLength 16).
    #[error("invalid hello nonce: {message}")]
    InvalidNonce { message: String },
    /// Cryptography-level failure (invalid key bytes, base64 decoding, …).
    #[error("crypto: {message}")]
    Crypto { message: String },
    /// RFC 8785 JCS canonicalization / serialization of the signed object
    /// failed.
    #[error("JCS canonicalization failed: {message}")]
    Jcs { message: String },
    /// A capability-token proof failed validation (malformed shape, bad
    /// signature, untrusted issuer, subject/audience/expiry mismatch, or
    /// claim-rule violation).
    #[error("capability token invalid: {message}")]
    TokenInvalid { message: String },
}

impl From<CoreErrorImpl> for CoreError {
    fn from(error: CoreErrorImpl) -> Self {
        match error {
            CoreErrorImpl::InvalidHelloSignature => Self::InvalidHelloSignature,
            CoreErrorImpl::NonceReplay => Self::NonceReplay,
            CoreErrorImpl::HandshakeFailed { reason } => Self::HandshakeFailed { reason },
            CoreErrorImpl::InvalidNonce(message) => Self::InvalidNonce { message },
            CoreErrorImpl::Crypto(message) => Self::Crypto { message },
            CoreErrorImpl::Jcs(message) => Self::Jcs { message },
            CoreErrorImpl::TokenInvalid(message) => Self::TokenInvalid { message },
        }
    }
}

/// FFI-facing mirror of [`crate::core::CoreInvokeError`] (invoke-path
/// sequence / correlation failures). Mapped 1:1 in
/// [`From<CoreInvokeErrorImpl>`].
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreInvokeError {
    /// The session's outbound sequence space (2⁵³−1) is exhausted; the
    /// session must be closed and reopened — sequences never wrap.
    #[error("sequence space exhausted — reopen session")]
    SequenceExhausted,
    /// An inbound invoke `sequence` is not the next expected one (replay or
    /// out-of-order); the invoke must not be dispatched.
    #[error("inbound sequence {actual} is not the next expected {expected}")]
    InboundSequenceMismatch { expected: u64, actual: i64 },
    /// A response did not echo the request's `session_id` / `sequence` /
    /// `request_id`.
    #[error("request/response mismatch")]
    CorrelationMismatch,
}

impl From<CoreInvokeErrorImpl> for CoreInvokeError {
    fn from(error: CoreInvokeErrorImpl) -> Self {
        match error {
            CoreInvokeErrorImpl::SequenceExhausted => Self::SequenceExhausted,
            CoreInvokeErrorImpl::InboundSequenceMismatch { expected, actual } => {
                Self::InboundSequenceMismatch { expected, actual }
            }
            CoreInvokeErrorImpl::CorrelationMismatch => Self::CorrelationMismatch,
        }
    }
}

/// Require `bytes` to be exactly 32 bytes (an Ed25519 secret or public key).
fn ed25519_key(bytes: Vec<u8>, what: &str) -> Result<[u8; 32], CoreError> {
    let len = bytes.len();
    bytes.try_into().map_err(|_| CoreError::Crypto {
        message: format!("expected a 32-byte {what}, got {len}"),
    })
}

/// Derive the wire `peer_id` string for a 32-byte Ed25519 public key.
///
/// The result matches rust-libp2p `PeerId::to_string()` for the same key
/// (locked by the golden-vector tests).
#[uniffi::export]
pub fn derive_peer_id_from_ed25519_pubkey(pubkey: Vec<u8>) -> Result<String, CoreError> {
    let pubkey = ed25519_key(pubkey, "Ed25519 public key")?;
    Ok(crate::core::derive_peer_id_from_ed25519_pubkey(&pubkey))
}

/// Sign a hello with a raw Ed25519 secret key (32 bytes), returning the
/// signed `ConnectHello` envelope as a JSON string.
///
/// `nonce` must meet the wire floor (minLength 16). `host_json` is the
/// canonical JSON of the `HostCapabilityManifest` embedded in
/// `ConnectHello.host`.
#[uniffi::export]
pub fn sign_hello_ed25519(
    secret: Vec<u8>,
    nonce: String,
    host_json: String,
) -> Result<String, CoreError> {
    let secret = ed25519_key(secret, "Ed25519 secret key")?;
    // The FFI boundary cannot carry the typed `HostCapabilityManifest`, so
    // it crosses as JSON; a parse failure is a malformed-input handshake
    // failure (the core takes the typed manifest and never sees raw JSON).
    let manifest: HostCapabilityManifest =
        serde_json::from_str(&host_json).map_err(|e| CoreError::HandshakeFailed {
            reason: format!("invalid host manifest JSON: {e}"),
        })?;
    let hello =
        crate::core::sign_hello_ed25519(&secret, &nonce, &manifest, None).map_err(CoreError::from)?;
    // Every envelope field is serializable, so a failure here is a
    // serialization defect — mapped to the canonicalization-family variant.
    serde_json::to_string(&hello).map_err(|e| CoreError::Jcs {
        message: format!("serialize hello: {e}"),
    })
}

/// Verify a received hello against a 32-byte Ed25519 public key.
///
/// `expected_peer_id` is the authenticated remote peer; `hello_json` is the
/// JSON string of the received `ConnectHello` envelope. Fails on protocol
/// version mismatch, public-key / peer-id binding mismatch, or an invalid
/// signature.
#[uniffi::export]
pub fn verify_hello_ed25519(
    public_key: Vec<u8>,
    expected_peer_id: String,
    hello_json: String,
) -> Result<(), CoreError> {
    let public_key = ed25519_key(public_key, "Ed25519 public key")?;
    let hello: ConnectHello =
        serde_json::from_str(&hello_json).map_err(|e| CoreError::HandshakeFailed {
            reason: format!("invalid hello JSON: {e}"),
        })?;
    crate::core::verify_hello_ed25519(&public_key, &expected_peer_id, &hello, None)
        .map_err(CoreError::from)
}

/// Single-use `(peer_id, nonce)` replay store — thread-safe FFI wrapper over
/// the core store.
#[derive(uniffi::Object)]
pub struct NonceStore {
    inner: Mutex<crate::core::NonceStore>,
}

#[uniffi::export]
impl NonceStore {
    /// Creates an empty store.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(crate::core::NonceStore::new()),
        })
    }

    /// Records `(peer_id, nonce)` unless it was already accepted; returns
    /// `false` on replay. Call only after the hello passed every earlier
    /// gate (allowlist, signature) so a rejected hello is not burned.
    #[must_use]
    pub fn check_and_record(&self, peer_id: String, nonce: String) -> bool {
        // Poisoning is unreachable today (no panicking payloads), but a
        // poisoned lock must not permanently brick the FFI object.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .check_and_record(&peer_id, &nonce)
    }
}

/// Whether `peer_id` is on the allowlist. Fails closed: an empty allowlist
/// rejects every peer.
#[uniffi::export]
#[must_use]
pub fn is_allowlisted(allowlist: Vec<String>, peer_id: String) -> bool {
    crate::core::is_allowlisted(&allowlist, &peer_id)
}

/// Outbound sequence counter — thread-safe FFI wrapper over the core
/// counter, starting at 0.
#[derive(uniffi::Object)]
pub struct OutboundSequence {
    inner: Mutex<crate::core::OutboundSequence>,
}

#[uniffi::export]
impl OutboundSequence {
    /// Creates a counter starting at 0 (the first allocate returns 0).
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(crate::core::OutboundSequence::new()),
        })
    }

    /// Assigns the next outbound sequence; on exhaustion (past the JSON-safe
    /// wire maximum) `SequenceExhausted` is returned and the counter stays
    /// exhausted — sequences never wrap. The caller must close the session.
    pub fn allocate(&self) -> Result<u64, CoreInvokeError> {
        // Poisoning is unreachable today (no panicking payloads), but a
        // poisoned lock must not permanently brick the FFI object.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .allocate()
            .map_err(CoreInvokeError::from)
    }
}

/// Inbound sequence expectation — thread-safe FFI wrapper over the core
/// expectation, starting at 0.
#[derive(uniffi::Object)]
pub struct InboundSequence {
    inner: Mutex<crate::core::InboundSequence>,
}

#[uniffi::export]
impl InboundSequence {
    /// Creates an expectation starting at 0 (the first accepted sequence).
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(crate::core::InboundSequence::new()),
        })
    }

    /// Accepts `sequence` iff it equals the next expected inbound sequence;
    /// on acceptance the expectation advances by 1 and the new expectation
    /// is returned. A replayed or out-of-order sequence yields
    /// `InboundSequenceMismatch` and the expectation is left unchanged — the
    /// caller must reject the invoke without dispatching it.
    pub fn advance(&self, sequence: i64) -> Result<u64, CoreInvokeError> {
        // Poisoning is unreachable today (no panicking payloads), but a
        // poisoned lock must not permanently brick the FFI object.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance(sequence)
            .map_err(CoreInvokeError::from)
    }
}

/// Checks that a response echoes the request's `session_id` / `sequence` /
/// `request_id` — the three echo fields, flattened to primitives.
#[uniffi::export]
pub fn check_response_correlation(
    expected_session_id: String,
    expected_sequence: u64,
    expected_request_id: String,
    actual_session_id: String,
    actual_sequence: u64,
    actual_request_id: String,
) -> Result<(), CoreInvokeError> {
    // Wire sequences are i64; a value above i64::MAX can never match a wire
    // echo, so it fails correlation.
    let expected_sequence =
        i64::try_from(expected_sequence).map_err(|_| CoreInvokeError::CorrelationMismatch)?;
    let actual_sequence =
        i64::try_from(actual_sequence).map_err(|_| CoreInvokeError::CorrelationMismatch)?;
    crate::core::check_response_correlation(
        &crate::core::Correlation {
            session_id: expected_session_id,
            sequence: expected_sequence,
            request_id: expected_request_id,
        },
        &crate::core::Correlation {
            session_id: actual_session_id,
            sequence: actual_sequence,
            request_id: actual_request_id,
        },
    )
    .map_err(CoreInvokeError::from)
}

/// Whether `op` may be dispatched in a session with
/// `negotiated_capabilities`. Fails closed: an unknown `op` has no core-table
/// requirement and is not authorized by this gate (hosts answer
/// `op_unsupported`).
#[uniffi::export]
#[must_use]
pub fn dispatch_allowed(op: String, negotiated_capabilities: Vec<String>) -> bool {
    crate::core::dispatch_allowed(&op, &negotiated_capabilities)
}

/// The capability required to dispatch `op`, per the protocol v1 core-op
/// table; `None` for product-defined ops.
#[uniffi::export]
#[must_use]
pub fn required_capability(op: String) -> Option<String> {
    crate::core::required_capability(&op).map(str::to_owned)
}

/// The connect protocol version exchanged in `ConnectHello` (protocol
/// version 1 is current).
#[uniffi::export]
#[must_use]
pub fn protocol_version() -> u64 {
    crate::core::PROTOCOL_VERSION
}

#[cfg(feature = "ffi")]
#[cfg(test)]
mod runtime_tests {
    use super::ffi_runtime;
    use std::thread;

    #[test]
    fn ffi_runtime_is_lazy_initialized_once_and_reused_across_threads() {
        let main_runtime = ffi_runtime();
        main_runtime.handle().block_on(async {});

        let main_addr = main_runtime as *const tokio::runtime::Runtime as usize;
        let handles: Vec<_> = (0..2)
            .map(|_| {
                thread::spawn(|| {
                    let thread_runtime = ffi_runtime();
                    thread_runtime.handle().block_on(async {});
                    thread_runtime as *const tokio::runtime::Runtime as usize
                })
            })
            .collect();

        for handle in handles {
            let thread_addr = handle.join().expect("thread joined");
            assert_eq!(main_addr, thread_addr);
            assert_eq!(main_addr, ffi_runtime() as *const tokio::runtime::Runtime as usize);
        }
    }
}

#[cfg(feature = "remote-adapter")]
#[cfg(test)]
mod foreign_transport_tests {
    use super::*;

    use crate::remote::transport::Transport as RemoteAsyncTransport;
    /// Rust impl of the callback [`Transport`] wrapping a loopback end — the
    /// same synchronous block-on-async surface a foreign binding uses. Sends
    /// and receives through the real in-repo loopback, so it exercises the
    /// callback → [`ForeignCallbackTransport`] bridge → async-trait path end
    /// to end (AR-2).
    struct LoopbackCallback {
        inner: crate::remote::transport::LoopbackTransport,
    }

    impl Transport for LoopbackCallback {
        fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.send(&envelope))
                .map_err(Into::into)
        }

        fn recv(&self) -> Result<Vec<u8>, TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.recv())
                .map_err(Into::into)
        }

        fn close(&self) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.close())
                .map_err(Into::into)
        }
    }

    #[test]
    fn foreign_callback_transport_round_trips_send_recv_close_through_bridge() {
        let pair = crate::remote::transport::loopback_transport_pair();
        // Each end is bridged through the async↔callback seam: the callback
        // is a Rust impl of the FFI `Transport`, exactly what a binding
        // provides.
        let client = ForeignCallbackTransport::new(Arc::new(LoopbackCallback {
            inner: pair.client,
        }));
        let server = ForeignCallbackTransport::new(Arc::new(LoopbackCallback {
            inner: pair.server,
        }));

        ffi_runtime().handle().block_on(async {
            // client → server
            client
                .send(b"hello from client")
                .await
                .expect("client send delivers to server recv");
            let got = server.recv().await.expect("server recv");
            assert_eq!(got, b"hello from client");

            // server → client
            server
                .send(b"reply from server")
                .await
                .expect("server send delivers to client recv");
            let got = client.recv().await.expect("client recv");
            assert_eq!(got, b"reply from server");

            // Close fails the peer's pending recv fast (AR-3): fails fast at
            // the transport level, the same error the adapter receive loop
            // converts into close_session.
            server.close().await.expect("server close");
            let err = client
                .recv()
                .await
                .expect_err("client recv after peer close fails fast");
            assert_eq!(err, crate::remote::transport::TransportError::Closed);
        });
    }

    #[test]
    fn ffi_loopback_pair_smoke_round_trips_send_recv_close() {
        // The binding-facing loopback object surface: send/recv/close via the
        // shared runtime, mirroring the callback trait's block-on-async shape.
        let pair = loopback_transport_pair();
        pair.client().send(b"ping".to_vec()).expect("client send");
        let got = pair.server().recv().expect("server recv");
        assert_eq!(got, b"ping");

        pair.client().close().expect("client close");
        let err = pair.server().recv().expect_err("server recv after close");
        assert_eq!(err, TransportError::Closed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::golden::{golden, golden_pubkey, golden_seed};

    /// Golden key pair (seed bytes 1..=32) — the same fixtures as the core
    /// golden-vector tests, so these wrapper tests assert byte parity with
    /// the core surface. Loaded from the shared cross-language fixture
    /// `tests/fixtures/golden-hello.json` (SSOT); the golden signature,
    /// peer id, and manifest JSON are the fixture's pinned output bytes.

    #[test]
    fn derive_peer_id_via_ffi_matches_golden() {
        assert_eq!(
            derive_peer_id_from_ed25519_pubkey(golden_pubkey().to_vec()).expect("derives"),
            golden().peer_id
        );
    }

    #[test]
    fn derive_peer_id_rejects_wrong_key_length() {
        let err = derive_peer_id_from_ed25519_pubkey(vec![0u8; 31]).expect_err("short key");
        assert!(matches!(err, CoreError::Crypto { .. }));
        let err = derive_peer_id_from_ed25519_pubkey(vec![0u8; 33]).expect_err("long key");
        assert!(matches!(err, CoreError::Crypto { .. }));
    }

    #[test]
    fn sign_hello_via_ffi_matches_golden_signature() {
        let hello_json = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            golden().manifest_json.clone(),
        )
        .expect("signs");
        let hello: ConnectHello = serde_json::from_str(&hello_json).expect("hello JSON parses");
        assert_eq!(hello.peer_id.as_str(), golden().peer_id.as_str());
        assert_eq!(hello.signature.as_str(), golden().signature_b64u.as_str());
    }

    #[test]
    fn sign_verify_round_trip_via_ffi_json() {
        let hello_json = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            golden().manifest_json.clone(),
        )
        .expect("signs");
        verify_hello_ed25519(
            golden_pubkey().to_vec(),
            golden().peer_id.clone(),
            hello_json,
        )
        .expect("verifies");
    }

    #[test]
    fn tampered_hello_json_fails_verification_with_mapped_error() {
        let hello_json = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            golden().manifest_json.clone(),
        )
        .expect("signs");
        let tampered = hello_json.replace("data-store", "checker");
        let err = verify_hello_ed25519(
            golden_pubkey().to_vec(),
            golden().peer_id.clone(),
            tampered,
        )
        .expect_err("tampered host");
        assert!(matches!(err, CoreError::InvalidHelloSignature));
    }

    #[test]
    fn malformed_hello_json_fails_verification_with_mapped_error() {
        // Non-JSON input and valid JSON with the wrong shape both fail
        // serde parsing, which the wrapper maps to a handshake failure —
        // the same mapping as malformed host manifest JSON on the sign path
        // (sign_hello_ed25519 also returns HandshakeFailed for unparseable
        // JSON), so sign and verify are consistent.
        for malformed in ["not json".to_owned(), r#"{"not":"a-hello"}"#.to_owned()] {
            let err = verify_hello_ed25519(
                golden_pubkey().to_vec(),
                golden().peer_id.clone(),
                malformed,
            )
            .expect_err("malformed hello");
            assert!(matches!(err, CoreError::HandshakeFailed { .. }));
        }
    }

    #[test]
    fn bad_nonce_and_bad_manifest_json_map_to_errors() {
        let err = sign_hello_ed25519(
            golden_seed().to_vec(),
            ["short"].join("-"),
            golden().manifest_json.clone(),
        )
        .expect_err("short nonce");
        assert!(matches!(err, CoreError::InvalidNonce { .. }));

        let err = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            "not json".to_owned(),
        )
        .expect_err("malformed manifest");
        assert!(matches!(err, CoreError::HandshakeFailed { .. }));
    }

    #[test]
    fn nonce_store_object_rejects_replay() {
        let store = NonceStore::new();
        assert!(store.check_and_record("peer-a".to_owned(), "nonce-1".to_owned()));
        assert!(!store.check_and_record("peer-a".to_owned(), "nonce-1".to_owned()));
        // Nonce scoping is per sender peer_id.
        assert!(store.check_and_record("peer-b".to_owned(), "nonce-1".to_owned()));
    }

    #[test]
    fn allowlist_check_via_ffi_fails_closed() {
        let allowlist = vec!["peer-a".to_owned(), "peer-b".to_owned()];
        assert!(is_allowlisted(allowlist.clone(), "peer-a".to_owned()));
        assert!(!is_allowlisted(allowlist, "peer-c".to_owned()));
        assert!(!is_allowlisted(Vec::new(), "peer-a".to_owned()));
    }

    #[test]
    fn sequence_objects_allocate_and_advance() {
        let outbound = OutboundSequence::new();
        assert_eq!(outbound.allocate().expect("first"), 0);
        assert_eq!(outbound.allocate().expect("second"), 1);

        let inbound = InboundSequence::new();
        assert_eq!(inbound.advance(0).expect("first"), 1);
        let err = inbound.advance(0).expect_err("replay");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 1,
                actual: 0
            }
        ));
    }

    #[test]
    fn outbound_sequence_exhaustion_maps_through_wrapper() {
        // Position the core counter at the wire maximum via the test-only
        // setter (no 2^53 allocations), then drive exhaustion through the
        // FFI wrapper to exercise the error mapping end to end.
        let mut core_seq = crate::core::OutboundSequence::new();
        core_seq.set_next(crate::core::MAX_SEQUENCE);
        let outbound = OutboundSequence {
            inner: Mutex::new(core_seq),
        };
        assert_eq!(
            outbound.allocate().expect("last valid"),
            crate::core::MAX_SEQUENCE
        );
        let err = outbound.allocate().expect_err("exhausted");
        assert!(matches!(err, CoreInvokeError::SequenceExhausted));
        // Still exhausted — no wrap-around.
        let err = outbound.allocate().expect_err("still exhausted");
        assert!(matches!(err, CoreInvokeError::SequenceExhausted));
    }

    #[test]
    fn correlation_check_via_flattened_primitives() {
        check_response_correlation(
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
        )
        .expect("exact echo passes");

        let err = check_response_correlation(
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            1,
            "req-1".to_owned(),
        )
        .expect_err("sequence mismatch");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));
    }

    #[test]
    fn correlation_sequences_above_i64_max_fail_guarded() {
        // Wire sequences are i64; a u64 above i64::MAX can never match a
        // wire echo, so the guard fails correlation on either side.
        let over_i64 = i64::MAX as u64 + 1;
        let err = check_response_correlation(
            "sess-1".to_owned(),
            over_i64,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
        )
        .expect_err("expected sequence above i64::MAX");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));

        let err = check_response_correlation(
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            over_i64,
            "req-1".to_owned(),
        )
        .expect_err("actual sequence above i64::MAX");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));
    }

    #[test]
    fn dispatch_gate_required_capability_and_protocol_version() {
        assert!(dispatch_allowed(
            "check".to_owned(),
            vec!["spoke-baseline".to_owned()]
        ));
        assert!(!dispatch_allowed(
            "check".to_owned(),
            vec!["l2-computable".to_owned()]
        ));
        assert!(!dispatch_allowed(
            "custom-op".to_owned(),
            vec!["spoke-baseline".to_owned()]
        ));
        assert_eq!(
            required_capability("check".to_owned()).as_deref(),
            Some("spoke-baseline")
        );
        assert_eq!(
            required_capability("project".to_owned()).as_deref(),
            Some("l2-computable")
        );
        assert_eq!(required_capability("custom-op".to_owned()), None);
        assert_eq!(protocol_version(), 1);
    }
}


#[cfg(all(test, feature = "remote-adapter"))]
mod remote_adapter_ffi_tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use serde_json::json;
    use spoke_fixture_toy_world::ToyWorldAdapter;
    use spoke_schemas::KnowledgeEntry;

    use crate::core::derive_peer_id_from_ed25519_pubkey;
    use crate::core::golden::{golden, golden_pubkey, golden_seed};
    use crate::remote::transport::Transport as RemoteAsyncTransport;

    use crate::test_support::loopback_oracle::{
        dial, fresh_entry, manifest, pubkey_client, pubkey_host, seed_client, seed_host,
        start_loopback_host, DialOptions, LoopbackHostOptions,
    };
    use super::foreign_transport::{ForeignCallbackTransport, Transport, TransportError};
    use super::remote_adapter_ffi::{
        connect_remote_adapter_ffi, FfiError, RemoteAdapterFFI,
    };
    use super::ffi_runtime;

    struct LoopbackCallback {
        inner: crate::remote::transport::LoopbackTransport,
    }

    impl Transport for LoopbackCallback {
        fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.send(&envelope))
                .map_err(Into::into)
        }

        fn recv(&self) -> Result<Vec<u8>, TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.recv())
                .map_err(Into::into)
        }

        fn close(&self) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.close())
                .map_err(Into::into)
        }
    }

    fn client_manifest_json() -> String {
        serde_json::to_string(&manifest("test-client", &["spoke-baseline"])).expect("manifest")
    }

    /// Callback transport that signals once after the first post-dial `send` succeeds
    /// (invoke request on the wire, waiting for host response).
    struct InvokeInFlightCallback {
        inner: crate::remote::transport::LoopbackTransport,
        in_flight_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    }

    impl Transport for InvokeInFlightCallback {
        fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError> {
            let result = ffi_runtime()
                .handle()
                .block_on(self.inner.send(&envelope))
                .map_err(Into::into);
            if result.is_ok() {
                if let Some(tx) = self.in_flight_tx.lock().expect("in-flight tx lock").take() {
                    let _ = tx.send(());
                }
            }
            result
        }

        fn recv(&self) -> Result<Vec<u8>, TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.recv())
                .map_err(Into::into)
        }

        fn close(&self) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.close())
                .map_err(Into::into)
        }
    }

    fn dial_ffi(client: crate::remote::transport::LoopbackTransport) -> Arc<RemoteAdapterFFI> {
        let callback = Box::new(LoopbackCallback { inner: client });
        let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
        connect_remote_adapter_ffi(
            callback,
            seed_client().to_vec(),
            client_manifest_json(),
            pubkey_host().to_vec(),
            vec![peer_id_host],
            None,
        )
        .expect("ffi dial")
    }

    #[test]
    fn connect_remote_adapter_ffi_rejects_allowlist_miss() {
        let pair = crate::remote::transport::loopback_transport_pair();
        let callback = Box::new(LoopbackCallback { inner: pair.client });
        let err = match connect_remote_adapter_ffi(
            callback,
            seed_client().to_vec(),
            client_manifest_json(),
            pubkey_host().to_vec(),
            vec![],
            None,
        ) {
            Err(err) => err,
            Ok(_) => panic!("empty allowlist fails closed"),
        };
        assert!(matches!(err, FfiError::Dial { kind, .. } if kind == "config"));
    }

    #[test]
    fn connect_remote_adapter_ffi_rejects_invalid_local_manifest_json() {
        let pair = crate::remote::transport::loopback_transport_pair();
        let callback = Box::new(LoopbackCallback { inner: pair.client });
        let err = match connect_remote_adapter_ffi(
            callback,
            seed_client().to_vec(),
            "{ not json".to_string(),
            pubkey_host().to_vec(),
            vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
            None,
        ) {
            Err(err) => err,
            Ok(_) => panic!("invalid manifest JSON must fail at dial"),
        };
        assert!(matches!(err, FfiError::Dial { kind, .. } if kind == "config"));
    }


    #[test]
    fn remote_adapter_ffi_put_get_round_trip_and_session_info() {
        let (async_client, host) = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            dial(host_adapter, DialOptions::default()).await
        });
        let ffi = RemoteAdapterFFI::from_adapter(async_client);

        assert_eq!(ffi.state(), "Established");
        assert_eq!(ffi.session_id().as_deref(), Some(host.session_id()));
        assert_eq!(
            ffi.remote_peer_id().as_deref(),
            Some(derive_peer_id_from_ed25519_pubkey(&pubkey_host()).as_str())
        );
        let remote_manifest = ffi.remote_manifest().expect("remote manifest json");
        let manifest: serde_json::Value = serde_json::from_str(&remote_manifest).expect("json");
        assert_eq!(manifest.get("host_id").and_then(|v| v.as_str()), Some("test-host"));

        let entry = fresh_entry("kb_ffi_cartographer", "FFI Cartographer");
        let entry_json = serde_json::to_string(&entry).expect("entry json");
        let put_json = ffi
            .put_knowledge_entry(entry_json, None)
            .expect("put succeeds");
        let put_entry: KnowledgeEntry = serde_json::from_str(&put_json).expect("put entry");
        assert_eq!(put_entry.entry_id.as_str(), "kb_ffi_cartographer");

        let get_json = ffi
            .get_knowledge_entry("kb_ffi_cartographer".to_string())
            .expect("get succeeds");
        let got: KnowledgeEntry = serde_json::from_str(&get_json).expect("get entry");
        assert_eq!(got.entry_id.as_str(), "kb_ffi_cartographer");

        host.close();
    }

    #[test]
    fn connect_remote_adapter_ffi_dials_over_foreign_callback_transport() {
        let (client, host) = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            let pair = crate::remote::transport::loopback_transport_pair();
            let host = crate::test_support::loopback_oracle::start_loopback_host(
                crate::test_support::loopback_oracle::LoopbackHostOptions {
                    transport: Arc::new(pair.server),
                    host_seed: seed_host(),
                    host_manifest: manifest("test-host", &["spoke-baseline"]),
                    allowlist: vec![derive_peer_id_from_ed25519_pubkey(
                        &crate::test_support::loopback_oracle::pubkey_client(),
                    )],
                    adapter: Arc::new(host_adapter),
                    delay: Box::new(|_| 0),
                    response_override: None,
                    session_peer_ids: None,
                },
            )
            .await;

            (pair.client, host)
        });
        let ffi = dial_ffi(client);

        assert_eq!(ffi.state(), "Established");
        assert_eq!(
            ffi.remote_peer_id().as_deref(),
            Some(derive_peer_id_from_ed25519_pubkey(&pubkey_host()).as_str())
        );

        let entry = fresh_entry("kb_ffi_callback", "Callback Cartographer");
        let entry_json = serde_json::to_string(&entry).expect("entry json");
        ffi.put_knowledge_entry(entry_json, None)
            .expect("put over ffi dial");
        ffi.get_knowledge_entry("kb_ffi_callback".to_string())
            .expect("get over ffi dial");
        ffi.close();
        drop(host);
    }

    #[test]
    fn remote_adapter_ffi_close_rejects_port_calls_with_session_closed() {
        let (async_client, host) = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            dial(host_adapter, DialOptions::default()).await
        });
        let ffi = RemoteAdapterFFI::from_adapter(async_client);
        ffi.close();
        assert_eq!(ffi.state(), "Closed");
        let err = ffi
            .get_knowledge_entry("kb_tw_mira".to_string())
            .expect_err("post-close port call rejects");
        assert!(matches!(
            err,
            FfiError::Rejected { code, kind: Some(kind), .. }
                if code == "INTERNAL_ERROR" && kind == "session_closed"
        ));
        host.close();
    }

    #[test]
    fn remote_adapter_ffi_close_closes_loopback_transport() {
        let pair = crate::remote::transport::loopback_transport_pair();
        let peer_client = pair.client.clone();
        let peer_server = pair.server.clone();
        let host = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            start_loopback_host(LoopbackHostOptions {
                transport: Arc::new(pair.server),
                host_seed: seed_host(),
                host_manifest: manifest("test-host", &["spoke-baseline"]),
                allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
                adapter: Arc::new(host_adapter),
                delay: Box::new(|_| 0),
                response_override: None,
                session_peer_ids: None,
            })
            .await
        });
        let ffi = dial_ffi(pair.client);
        assert_eq!(ffi.state(), "Established");
        ffi.close();
        assert_eq!(ffi.state(), "Closed");

        let client_recv = ffi_runtime().block_on(peer_client.recv());
        assert!(
            matches!(
                client_recv,
                Err(crate::remote::transport::TransportError::Closed)
            ),
            "client transport should report closed after ffi close: {client_recv:?}"
        );
        let server_recv = ffi_runtime().block_on(peer_server.recv());
        assert!(
            matches!(
                server_recv,
                Err(crate::remote::transport::TransportError::Closed)
            ),
            "server transport should report closed after ffi close: {server_recv:?}"
        );
        host.close();
    }


    #[test]
    fn remote_adapter_ffi_concurrent_invokes_from_os_threads() {
        let (async_client, host) = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            dial(host_adapter, DialOptions::default()).await
        });
        let ffi = Arc::new(RemoteAdapterFFI::from_adapter(async_client));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let ffi = Arc::clone(&ffi);
                thread::spawn(move || {
                    ffi.get_knowledge_entry("kb_tw_mira".to_string())
                        .expect("concurrent get")
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread join");
        }
        host.close();
    }

    // ── D7 parity gate: async adapter ↔ FFI error surface (AR-5) ───────────

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde_json::Value;
    use spoke_operations::{KnowledgeEntryPort, SpokeReject, SpokeRejectCode, SpokeResult};

    use crate::remote::{
        connect_remote_adapter, loopback_transport_pair, RemoteAdapterError, RemoteAdapterOptions,
        RemoteIdentity,
    };
    use crate::test_support::loopback_oracle::LoopbackHost;

    struct AsyncCallbackTransport {
        inner: Arc<dyn RemoteAsyncTransport>,
    }

    impl Transport for AsyncCallbackTransport {
        fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.send(&envelope))
                .map_err(Into::into)
        }

        fn recv(&self) -> Result<Vec<u8>, TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.recv())
                .map_err(Into::into)
        }

        fn close(&self) -> Result<(), TransportError> {
            ffi_runtime()
                .handle()
                .block_on(self.inner.close())
                .map_err(Into::into)
        }
    }

    struct TamperCallbackTransport {
        inner: Arc<dyn RemoteAsyncTransport>,
        outbound: Option<Arc<dyn Fn(Value) -> Option<Value> + Send + Sync>>,
    }

    #[async_trait]
    impl RemoteAsyncTransport for TamperCallbackTransport {
        async fn send(&self, envelope: &[u8]) -> Result<(), crate::remote::transport::TransportError> {
            let Some(mutate) = self.outbound.as_ref() else {
                return self.inner.send(envelope).await;
            };
            let doc: Value = serde_json::from_slice(envelope)
                .map_err(|_| crate::remote::transport::TransportError::Closed)?;
            match mutate(doc) {
                Some(mutated) => {
                    let bytes = serde_json::to_vec(&mutated)
                        .map_err(|_| crate::remote::transport::TransportError::Closed)?;
                    self.inner.send(&bytes).await
                }
                None => self.inner.send(envelope).await,
            }
        }

        async fn recv(&self) -> Result<Vec<u8>, crate::remote::transport::TransportError> {
            self.inner.recv().await
        }

        async fn close(&self) -> Result<(), crate::remote::transport::TransportError> {
            self.inner.close().await
        }
    }

    struct FailSendAfter {
        inner: Arc<dyn RemoteAsyncTransport>,
        count: AtomicUsize,
        fail_after: usize,
    }

    #[async_trait]
    impl RemoteAsyncTransport for FailSendAfter {
        async fn send(&self, envelope: &[u8]) -> Result<(), crate::remote::transport::TransportError> {
            let n = self.count.fetch_add(1, Ordering::SeqCst);
            if n >= self.fail_after {
                return Err(crate::remote::transport::TransportError::Io(
                    "injected transport send failure".into(),
                ));
            }
            self.inner.send(envelope).await
        }

        async fn recv(&self) -> Result<Vec<u8>, crate::remote::transport::TransportError> {
            self.inner.recv().await
        }

        async fn close(&self) -> Result<(), crate::remote::transport::TransportError> {
            self.inner.close().await
        }
    }

    struct HangingRecvTransport {
        state: Arc<HangingRecvState>,
    }

    struct HangingRecvState {
        closed: std::sync::atomic::AtomicBool,
        parked: Mutex<Vec<std::thread::Thread>>,
    }

    impl HangingRecvTransport {
        fn new() -> Self {
            Self {
                state: Arc::new(HangingRecvState {
                    closed: std::sync::atomic::AtomicBool::new(false),
                    parked: Mutex::new(Vec::new()),
                }),
            }
        }

        fn parked_thread_count(&self) -> usize {
            self.state.parked.lock().expect("parked lock").len()
        }
    }

    struct HangingRecvTransportHolder(Arc<HangingRecvTransport>);

    impl Transport for HangingRecvTransportHolder {
        fn send(&self, _envelope: Vec<u8>) -> Result<(), TransportError> {
            self.0.send(_envelope)
        }

        fn recv(&self) -> Result<Vec<u8>, TransportError> {
            self.0.recv()
        }

        fn close(&self) -> Result<(), TransportError> {
            self.0.close()
        }
    }

    impl Transport for HangingRecvTransport {
        fn send(&self, _envelope: Vec<u8>) -> Result<(), TransportError> {
            Ok(())
        }

        fn recv(&self) -> Result<Vec<u8>, TransportError> {
            if self.state.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            let current = std::thread::current();
            self.state
                .parked
                .lock()
                .expect("parked lock")
                .push(current);
            if self.state.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            std::thread::park();
            if self.state.closed.load(std::sync::atomic::Ordering::Acquire) {
                Err(TransportError::Closed)
            } else {
                unreachable!("parked recv should not return without close")
            }
        }

        fn close(&self) -> Result<(), TransportError> {
            self.state
                .closed
                .store(true, std::sync::atomic::Ordering::Release);
            let parked = std::mem::take(&mut *self.state.parked.lock().expect("parked lock"));
            for thread in parked {
                thread.unpark();
            }
            Ok(())
        }
    }

    struct HangingAsyncRecvTransport;

    #[async_trait]
    impl RemoteAsyncTransport for HangingAsyncRecvTransport {
        async fn send(&self, _envelope: &[u8]) -> Result<(), crate::remote::transport::TransportError> {
            Ok(())
        }

        async fn recv(&self) -> Result<Vec<u8>, crate::remote::transport::TransportError> {
            std::future::pending().await
        }

        async fn close(&self) -> Result<(), crate::remote::transport::TransportError> {
            Ok(())
        }
    }

    fn reject_kind(result: &SpokeResult<KnowledgeEntry>) -> Option<&str> {
        match result {
            SpokeResult::Reject(reject) => reject
                .details
                .as_ref()
                .and_then(|details| details.get("kind"))
                .and_then(Value::as_str),
            SpokeResult::Ok(_) => None,
        }
    }

    fn reject_wire_code(result: &SpokeResult<KnowledgeEntry>) -> Option<&str> {
        match result {
            SpokeResult::Reject(reject) => reject
                .details
                .as_ref()
                .and_then(|details| details.get("wire_code"))
                .and_then(Value::as_str),
            SpokeResult::Ok(_) => None,
        }
    }

    fn assert_ffi_matches_spoke_reject(ffi_err: FfiError, reject: &SpokeReject) {
        match ffi_err {
            FfiError::Rejected {
                code,
                kind,
                wire_code,
                ..
            } => {
                assert_eq!(code, reject.code.as_str());
                let expected_kind = reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let expected_wire = reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                assert_eq!(kind, expected_kind);
                assert_eq!(wire_code, expected_wire);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    fn parity_on_same_adapter<F>(options: DialOptions, invoke: F)
    where
        F: FnOnce(
            Arc<crate::remote::RemoteAdapter>,
            Arc<RemoteAdapterFFI>,
        ) -> (SpokeResult<KnowledgeEntry>, FfiError),
    {
        let (client, host) = ffi_runtime().block_on(async {
            dial(ToyWorldAdapter::with_committed_fixtures(), options).await
        });
        let ffi = RemoteAdapterFFI::from_adapter(Arc::clone(&client));
        let (async_result, ffi_err) = invoke(client, ffi);
        match &async_result {
            SpokeResult::Reject(reject) => assert_ffi_matches_spoke_reject(ffi_err, reject),
            SpokeResult::Ok(_) => panic!("async invoke must reject: {async_result:?}"),
        }
        host.close();
    }

    fn dial_ffi_with(options: DialOptions) -> (Arc<RemoteAdapterFFI>, LoopbackHost) {
        let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
        let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
        let pair = loopback_transport_pair();
        let (client, host) = ffi_runtime().block_on(async {
            let host = start_loopback_host(LoopbackHostOptions {
                transport: Arc::new(pair.server),
                host_seed: seed_host(),
                host_manifest: manifest("test-host", &["spoke-baseline"]),
                allowlist: options
                    .host_allowlist
                    .clone()
                    .unwrap_or_else(|| vec![peer_id_client.clone()]),
                adapter: Arc::new(ToyWorldAdapter::with_committed_fixtures()),
                delay: options
                    .host_delay
                    .unwrap_or_else(|| Box::new(|_| 0)),
                response_override: options.host_response_override,
                session_peer_ids: None,
            })
            .await;
            let client = match options.client_transport {
                Some(wrap) => wrap(Arc::new(pair.client)),
                None => Arc::new(pair.client) as Arc<dyn RemoteAsyncTransport>,
            };
            (client, host)
        });
        let callback = Box::new(AsyncCallbackTransport { inner: client });
        let manifest_json = serde_json::to_string(
            &options
                .client_manifest
                .unwrap_or_else(|| manifest("test-client", &["spoke-baseline"])),
        )
        .expect("manifest");
        let ffi = connect_remote_adapter_ffi(
            callback,
            seed_client().to_vec(),
            manifest_json,
            pubkey_host().to_vec(),
            options
                .allowlist
                .unwrap_or_else(|| vec![peer_id_host.clone()]),
            options.invoke_timeout_ms,
        )
        .expect("ffi dial");
        (ffi, host)
    }

    #[test]
    fn ffi_error_parity_invoke_timeout() {
        let delay_ms = Arc::new(AtomicUsize::new(100));
        let delay_flag = Arc::clone(&delay_ms);
        parity_on_same_adapter(
            DialOptions {
                invoke_timeout_ms: Some(20),
                host_delay: Some(Box::new(move |_| delay_flag.load(Ordering::Relaxed) as u64)),
                ..Default::default()
            },
            |client, ffi| {
                let async_result = ffi_runtime()
                    .block_on(client.get_knowledge_entry("kb_tw_mira"));
                let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
                    Err(err) => err,
                    Ok(_) => panic!("ffi invoke must reject"),
                };
                assert_eq!(reject_kind(&async_result), Some("timeout"));
                (async_result, ffi_err)
            },
        );
    }

    #[test]
    fn ffi_error_parity_session_closed_mid_flight() {
        parity_on_same_adapter(
            DialOptions {
                host_delay: Some(Box::new(|_| 100)),
                ..Default::default()
            },
            |client, ffi| {
                let async_pending = client.get_knowledge_entry("kb_tw_mira");
                ffi_runtime().block_on(async {
                    tokio::task::yield_now().await;
                });
                client.close();
                let async_result = ffi_runtime().block_on(async_pending);
                let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
                    Err(err) => err,
                    Ok(_) => panic!("ffi invoke must reject"),
                };
                (async_result, ffi_err)
            },
        );
        // Covered by `parity_on_same_adapter` assert + session_closed D7 row in unit tests.
    }

    #[test]
    fn ffi_error_parity_transport_io_on_invoke() {
        let fail_send = || {
            Box::new(|client_end: Arc<dyn RemoteAsyncTransport>| {
                Arc::new(FailSendAfter {
                    inner: client_end,
                    count: AtomicUsize::new(0),
                    fail_after: 1,
                }) as Arc<dyn RemoteAsyncTransport>
            }) as Box<dyn Fn(Arc<dyn RemoteAsyncTransport>) -> Arc<dyn RemoteAsyncTransport> + Send + Sync>
        };
        let (async_result, host) = ffi_runtime().block_on(async {
            let (client, host) = dial(
                ToyWorldAdapter::with_committed_fixtures(),
                DialOptions {
                    client_transport: Some(fail_send()),
                    ..Default::default()
                },
            ).await;
            let result = client.get_knowledge_entry("kb_tw_mira").await;
            (result, host)
        });
        let (ffi, host2) = dial_ffi_with(DialOptions {
            client_transport: Some(fail_send()),
            ..Default::default()
        });
        let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
            Err(err) => err,
            Ok(_) => panic!("ffi invoke must reject"),
        };
        match &async_result {
            SpokeResult::Reject(reject) => assert_ffi_matches_spoke_reject(ffi_err, reject),
            SpokeResult::Ok(_) => panic!("async invoke must reject"),
        }
        assert_eq!(reject_kind(&async_result), Some("transport"));
        host.close();
        host2.close();
    }

    #[test]
    fn ffi_error_parity_correlation_mismatch() {
        let mangled_async = Arc::new(AtomicBool::new(true));
        let mangled_async_flag = Arc::clone(&mangled_async);
        let mangled_ffi = Arc::new(AtomicBool::new(true));
        let mangled_ffi_flag = Arc::clone(&mangled_ffi);
        let override_box = |flag: Arc<AtomicBool>| {
            Box::new(move |request: &spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest| {
                if !flag.swap(false, Ordering::SeqCst) {
                    return None;
                }
                Some(json!({
                    "session_id": request.session_id,
                    "sequence": request.sequence + 1,
                    "request_id": request.request_id,
                    "payload": {},
                    "extensions": {},
                }))
            })
        };
        let (async_result, host) = ffi_runtime().block_on(async {
            let (client, host) = dial(
                ToyWorldAdapter::with_committed_fixtures(),
                DialOptions {
                    host_response_override: Some(override_box(mangled_async_flag)),
                    ..Default::default()
                },
            )
            .await;
            let result = client.get_knowledge_entry("kb_tw_mira").await;
            (result, host)
        });
        let (ffi, host2) = dial_ffi_with(DialOptions {
            host_response_override: Some(override_box(mangled_ffi_flag)),
            ..Default::default()
        });
        let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
            Err(err) => err,
            Ok(value) => panic!("ffi invoke must reject, got {value}"),
        };
        match &async_result {
            SpokeResult::Reject(reject) => assert_ffi_matches_spoke_reject(ffi_err, reject),
            SpokeResult::Ok(_) => panic!("async invoke must reject"),
        }
        assert_eq!(reject_kind(&async_result), Some("correlation_mismatch"));
        host.close();
        host2.close();
    }

    #[test]
    fn ffi_error_parity_dispatch_deny_op_unsupported() {
        let no_baseline = manifest("test-client", &["l2-computable"]);
        parity_on_same_adapter(
            DialOptions {
                client_manifest: Some(no_baseline),
                ..Default::default()
            },
            |client, ffi| {
                let async_result = ffi_runtime()
                    .block_on(client.get_knowledge_entry("kb_tw_mira"));
                let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
                    Err(err) => err,
                    Ok(_) => panic!("ffi invoke must reject"),
                };
                assert_eq!(reject_wire_code(&async_result), Some("op_unsupported"));
                (async_result, ffi_err)
            },
        );
    }

    #[test]
    fn ffi_error_parity_envelope_auth_missing() {
        let strip_signature = || {
            let stripped = Arc::new(AtomicBool::new(false));
            let stripped_flag = Arc::clone(&stripped);
            (
                stripped,
                Box::new(move |client_end: Arc<dyn RemoteAsyncTransport>| {
                    let stripped_flag = Arc::clone(&stripped_flag);
                    Arc::new(TamperCallbackTransport {
                        inner: client_end,
                        outbound: Some(Arc::new(move |doc| {
                            if doc.get("op").is_some() && !stripped_flag.swap(true, Ordering::SeqCst) {
                                let mut doc = doc;
                                if let Some(object) = doc.as_object_mut() {
                                    object.remove("signature");
                                }
                                return Some(doc);
                            }
                            None
                        })),
                    }) as Arc<dyn RemoteAsyncTransport>
                }) as Box<dyn Fn(Arc<dyn RemoteAsyncTransport>) -> Arc<dyn RemoteAsyncTransport> + Send + Sync>,
            )
        };
        let (stripped_async, transport_async) = strip_signature();
        let (stripped_ffi, transport_ffi) = strip_signature();
        let _ = (stripped_async, stripped_ffi);
        let (async_result, host) = ffi_runtime().block_on(async {
            let (client, host) = dial(
                ToyWorldAdapter::with_committed_fixtures(),
                DialOptions {
                    client_transport: Some(transport_async),
                    ..Default::default()
                },
            ).await;
            let result = client.get_knowledge_entry("kb_tw_mira").await;
            (result, host)
        });
        let (ffi, host2) = dial_ffi_with(DialOptions {
            client_transport: Some(transport_ffi),
            ..Default::default()
        });
        let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
            Err(err) => err,
            Ok(_) => panic!("ffi invoke must reject"),
        };
        match &async_result {
            SpokeResult::Reject(reject) => assert_ffi_matches_spoke_reject(ffi_err, reject),
            SpokeResult::Ok(_) => panic!("async invoke must reject"),
        }
        assert_eq!(reject_kind(&async_result), Some("envelope_auth_missing"));
        host.close();
        host2.close();
    }

    #[test]
    fn ffi_error_parity_envelope_auth_invalid() {
        let tamper_payload = || {
            let tampered = Arc::new(AtomicBool::new(false));
            let tampered_flag = Arc::clone(&tampered);
            (
                tampered,
                Box::new(move |client_end: Arc<dyn RemoteAsyncTransport>| {
                    let tampered_flag = Arc::clone(&tampered_flag);
                    Arc::new(TamperCallbackTransport {
                        inner: client_end,
                        outbound: Some(Arc::new(move |doc| {
                            if doc.get("op").is_some() && !tampered_flag.swap(true, Ordering::SeqCst) {
                                let mut doc = doc;
                                doc["payload"]["tampered"] = json!(true);
                                return Some(doc);
                            }
                            None
                        })),
                    }) as Arc<dyn RemoteAsyncTransport>
                }) as Box<dyn Fn(Arc<dyn RemoteAsyncTransport>) -> Arc<dyn RemoteAsyncTransport> + Send + Sync>,
            )
        };
        let (tampered_async, transport_async) = tamper_payload();
        let (tampered_ffi, transport_ffi) = tamper_payload();
        let _ = (tampered_async, tampered_ffi);
        let (async_result, host) = ffi_runtime().block_on(async {
            let (client, host) = dial(
                ToyWorldAdapter::with_committed_fixtures(),
                DialOptions {
                    client_transport: Some(transport_async),
                    ..Default::default()
                },
            ).await;
            let result = client.get_knowledge_entry("kb_tw_mira").await;
            (result, host)
        });
        let (ffi, host2) = dial_ffi_with(DialOptions {
            client_transport: Some(transport_ffi),
            ..Default::default()
        });
        let ffi_err = match ffi.get_knowledge_entry("kb_tw_mira".to_string()) {
            Err(err) => err,
            Ok(_) => panic!("ffi invoke must reject"),
        };
        match &async_result {
            SpokeResult::Reject(reject) => assert_ffi_matches_spoke_reject(ffi_err, reject),
            SpokeResult::Ok(_) => panic!("async invoke must reject"),
        }
        assert_eq!(reject_kind(&async_result), Some("envelope_auth_invalid"));
        host.close();
        host2.close();
    }

    #[test]
    fn ffi_error_parity_application_revision_conflict() {
        // `kb_tw_mira` is already present in committed fixtures; create-without-revision must reject.
        let duplicate = fresh_entry("kb_tw_mira", "Duplicate Mira");
        let duplicate_json = serde_json::to_string(&duplicate).expect("entry json");
        parity_on_same_adapter(DialOptions::default(), |client, ffi| {
            let async_result = ffi_runtime()
                .block_on(client.put_knowledge_entry(duplicate.clone(), None));
            let ffi_err = match ffi.put_knowledge_entry(duplicate_json, None) {
                Err(err) => err,
                Ok(_) => panic!("ffi invoke must reject"),
            };
            assert!(matches!(
                &async_result,
                SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::RevisionConflict
            ));
            (async_result, ffi_err)
        });
    }

    #[test]
    fn ffi_error_parity_application_knowledge_entry_not_found() {
        parity_on_same_adapter(DialOptions::default(), |client, ffi| {
            let async_result = ffi_runtime()
                .block_on(client.get_knowledge_entry("kb_ffi_missing_entry"));
            let ffi_err = match ffi.get_knowledge_entry("kb_ffi_missing_entry".to_string()) {
                Err(err) => err,
                Ok(value) => panic!("ffi invoke must reject, got {value}"),
            };
            assert!(matches!(
                &async_result,
                SpokeResult::Reject(reject)
                    if reject.code == SpokeRejectCode::KnowledgeEntryNotFound
            ));
            (async_result, ffi_err)
        });
    }

    #[test]
    fn ffi_error_parity_dial_handshake_when_host_rejects_hello() {
        let other_peer = derive_peer_id_from_ed25519_pubkey(&[0x20u8; 32]);
        let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
        let async_err = ffi_runtime().block_on(async {
            let pair = loopback_transport_pair();
            let host = start_loopback_host(LoopbackHostOptions {
                transport: Arc::new(pair.server),
                host_seed: seed_host(),
                host_manifest: manifest("test-host", &["spoke-baseline"]),
                allowlist: vec![other_peer.clone()],
                adapter: Arc::new(ToyWorldAdapter::with_committed_fixtures()),
                delay: Box::new(|_| 0),
                response_override: None,
                session_peer_ids: None,
            })
            .await;
            let err = match connect_remote_adapter(RemoteAdapterOptions {
                transport: Arc::new(pair.client),
                local_identity: RemoteIdentity {
                    seed: seed_client(),
                },
                local_manifest: manifest("test-client", &["spoke-baseline"]),
                remote_pubkey: pubkey_host(),
                allowlist: vec![peer_id_host.clone()],
                invoke_timeout_ms: Some(2000),
                capability_token: None,
            })
            .await
            {
                Err(error) => error,
                Ok(_) => panic!("async dial must fail handshake"),
            };
            host.close();
            err
        });
        let pair = loopback_transport_pair();
        let host = ffi_runtime().block_on(async {
            start_loopback_host(LoopbackHostOptions {
                transport: Arc::new(pair.server),
                host_seed: seed_host(),
                host_manifest: manifest("test-host", &["spoke-baseline"]),
                allowlist: vec![other_peer],
                adapter: Arc::new(ToyWorldAdapter::with_committed_fixtures()),
                delay: Box::new(|_| 0),
                response_override: None,
                session_peer_ids: None,
            })
            .await
        });
        let ffi_err = match connect_remote_adapter_ffi(
            Box::new(LoopbackCallback {
                inner: pair.client,
            }),
            seed_client().to_vec(),
            serde_json::to_string(&manifest("test-client", &["spoke-baseline"])).expect("manifest"),
            pubkey_host().to_vec(),
            vec![peer_id_host],
            Some(2000),
        ) {
            Err(error) => error,
            Ok(_) => panic!("ffi dial must fail handshake"),
        };
        assert!(matches!(async_err, RemoteAdapterError::Handshake(_)));
        assert!(matches!(ffi_err, FfiError::Dial { kind, .. } if kind == "handshake"));
        host.close();
    }

    #[test]
    fn ffi_error_parity_dial_timeout_on_never_responding_server() {
        let async_err = ffi_runtime().block_on(async {
            match connect_remote_adapter(RemoteAdapterOptions {
                transport: Arc::new(HangingAsyncRecvTransport),
                local_identity: RemoteIdentity {
                    seed: seed_client(),
                },
                local_manifest: manifest("test-client", &["spoke-baseline"]),
                remote_pubkey: pubkey_host(),
                allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
                invoke_timeout_ms: Some(50),
                capability_token: None,
            })
            .await
            {
                Err(error) => error,
                Ok(_) => panic!("async dial must timeout"),
            }
        });
        let hanging = Arc::new(HangingRecvTransport::new());
        let ffi_err = match connect_remote_adapter_ffi(
            Box::new(HangingRecvTransportHolder(Arc::clone(&hanging))),
            seed_client().to_vec(),
            serde_json::to_string(&manifest("test-client", &["spoke-baseline"])).expect("manifest"),
            pubkey_host().to_vec(),
            vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
            Some(50),
        ) {
            Err(error) => error,
            Ok(_) => panic!("ffi dial must timeout"),
        };
        assert!(matches!(async_err, RemoteAdapterError::Timeout(_)));
        assert!(matches!(ffi_err, FfiError::Dial { kind, .. } if kind == "timeout"));
        hanging.close().expect("close unparks blocked recv");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            hanging.parked_thread_count(),
            0,
            "HangingRecvTransport must not leak parked threads after close"
        );
    }

    #[test]
    fn remote_adapter_ffi_invoke_future_panic_surfaces_as_ffi_rejected() {
        let (async_client, host) = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            dial(host_adapter, DialOptions::default()).await
        });
        let ffi = RemoteAdapterFFI::from_adapter(async_client);
        super::remote_adapter_ffi::inject_panic_on_next_block_on_for_test();
        let err = ffi
            .get_knowledge_entry("kb_tw_mira".to_string())
            .expect_err("panicking future must not unwind across ffi");
        assert!(matches!(
            err,
            FfiError::Rejected {
                code,
                kind: Some(kind),
                wire_code: None,
                ..
            } if code == "INTERNAL_ERROR" && kind == "panic"
        ));
        host.close();
    }

    #[test]
    fn ffi_callback_transport_mid_flight_close_rejects_pending_invoke_session_closed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;

        use crate::remote::{
            connect_remote_adapter, RemoteAdapterOptions, RemoteIdentity,
        };

        let delay_active = Arc::new(AtomicBool::new(true));
        let delay_active_host = Arc::clone(&delay_active);
        let pair = crate::remote::transport::loopback_transport_pair();
        let in_flight_tx_slot = Arc::new(Mutex::new(None));
        let bridge = Arc::new(ForeignCallbackTransport::new(Arc::new(InvokeInFlightCallback {
            inner: pair.client,
            in_flight_tx: Arc::clone(&in_flight_tx_slot),
        })));
        let bridge_for_close = Arc::clone(&bridge);
        let host = ffi_runtime().block_on(async {
            start_loopback_host(LoopbackHostOptions {
                transport: Arc::new(pair.server),
                host_seed: seed_host(),
                host_manifest: manifest("test-host", &["spoke-baseline"]),
                allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
                adapter: Arc::new(ToyWorldAdapter::with_committed_fixtures()),
                delay: Box::new(move |_| {
                    if delay_active_host.load(Ordering::Relaxed) {
                        200
                    } else {
                        0
                    }
                }),
                response_override: None,
                session_peer_ids: None,
            })
            .await
        });
        let adapter = ffi_runtime()
            .block_on(connect_remote_adapter(RemoteAdapterOptions {
                transport: bridge,
                local_identity: RemoteIdentity {
                    seed: seed_client(),
                },
                local_manifest: serde_json::from_str(&client_manifest_json())
                    .expect("client manifest"),
                remote_pubkey: pubkey_host(),
                allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
                invoke_timeout_ms: None,
                capability_token: None,
            }))
            .expect("dial over ForeignCallbackTransport bridge");
        let ffi = Arc::new(RemoteAdapterFFI::from_adapter(adapter));
        let (invoke_in_flight_tx, invoke_in_flight_rx) = mpsc::channel();
        *in_flight_tx_slot.lock().expect("in-flight tx slot lock") = Some(invoke_in_flight_tx);
        let invoke_slot = Arc::new(Mutex::new(None));
        let invoke_slot_thread = Arc::clone(&invoke_slot);
        let ffi_thread = Arc::clone(&ffi);
        let invoke_handle = thread::spawn(move || {
            let result = ffi_thread.get_knowledge_entry("kb_tw_mira".to_string());
            *invoke_slot_thread.lock().expect("invoke slot lock") = Some(result);
        });
        invoke_in_flight_rx
            .recv()
            .expect("invoke send must complete before close");
        ffi_runtime()
            .block_on(bridge_for_close.close())
            .expect("close via ForeignCallbackTransport spawn_blocking path");
        invoke_handle.join().expect("invoke thread joined");
        let err = invoke_slot
            .lock()
            .expect("invoke slot lock")
            .take()
            .expect("invoke result")
            .expect_err("pending invoke must reject when transport closes");
        assert!(matches!(
            err,
            FfiError::Rejected {
                code,
                kind: Some(kind),
                ..
            } if code == "INTERNAL_ERROR" && kind == "session_closed"
        ));
        delay_active.store(false, Ordering::Relaxed);
        host.close();
    }

    #[test]
    fn hanging_recv_transport_releases_blocked_thread_on_close() {
        use std::time::Duration;

        let transport = Arc::new(HangingRecvTransport::new());
        let transport_recv = Arc::clone(&transport);
        let recv_handle = thread::spawn(move || transport_recv.recv());
        thread::sleep(Duration::from_millis(20));
        assert!(
            !recv_handle.is_finished(),
            "recv should block until transport close"
        );
        transport.close().expect("transport close unparks recv");
        let recv_result = recv_handle.join().expect("recv thread joined");
        assert!(matches!(recv_result, Err(TransportError::Closed)));
        assert_eq!(
            transport.parked_thread_count(),
            0,
            "no parked threads should remain after close"
        );
    }

    #[test]
    fn golden_hello_peer_id_matches_ffi_session_after_callback_dial() {
        let (client, host) = ffi_runtime().block_on(async {
            let host_adapter = ToyWorldAdapter::with_committed_fixtures();
            let pair = crate::remote::transport::loopback_transport_pair();
            let host = crate::test_support::loopback_oracle::start_loopback_host(
                crate::test_support::loopback_oracle::LoopbackHostOptions {
                    transport: Arc::new(pair.server),
                    host_seed: golden_seed(),
                    host_manifest: manifest("golden-host", &["spoke-baseline"]),
                    allowlist: vec![derive_peer_id_from_ed25519_pubkey(&golden_pubkey())],
                    adapter: Arc::new(host_adapter),
                    delay: Box::new(|_| 0),
                    response_override: None,
                    session_peer_ids: None,
                },
            )
            .await;

            (pair.client, host)
        });
        let ffi = connect_remote_adapter_ffi(
            Box::new(LoopbackCallback { inner: client }),
            golden_seed().to_vec(),
            serde_json::to_string(&manifest("golden-client", &["spoke-baseline"]))
                .expect("manifest"),
            golden_pubkey().to_vec(),
            vec![golden().peer_id.clone()],
            None,
        )
        .expect("golden ffi dial");
        assert_eq!(ffi.remote_peer_id().as_deref(), Some(golden().peer_id.as_str()));
        ffi.close();
        drop(host);
    }
}

#[cfg(all(test, feature = "remote-adapter"))]
mod multi_peer_router_ffi_tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;
    use spoke_fixture_toy_world::ToyWorldAdapter;

    use crate::core::derive_peer_id_from_ed25519_pubkey;
    use crate::remote::{
        connect_remote_adapter, RemoteAdapter, RemoteAdapterOptions, RemoteIdentity,
    };
    use crate::test_support::loopback_oracle::{
        fresh_entry, manifest, pubkey_client, seed_client, start_loopback_host,
        upsert_request, LoopbackHost, LoopbackHostOptions,
    };

    use super::ffi_runtime;
    use super::multi_peer_router_ffi::{new_multi_peer_router_ffi, MultiPeerRouterFFI};
    use super::remote_adapter_ffi::{FfiError, RemoteAdapterFFI};

    async fn dial_peer(
        host_seed: [u8; 32],
        host_manifest: spoke_schemas::HostCapabilityManifest,
    ) -> (Arc<RemoteAdapter>, LoopbackHost) {
        let host_pubkey = SigningKey::from_bytes(&host_seed)
            .verifying_key()
            .to_bytes();
        let peer_id_host = derive_peer_id_from_ed25519_pubkey(&host_pubkey);
        let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

        let pair = crate::remote::transport::loopback_transport_pair();
        let host = start_loopback_host(LoopbackHostOptions {
            transport: Arc::new(pair.server),
            host_seed,
            host_manifest,
            allowlist: vec![peer_id_client.clone()],
            adapter: Arc::new(ToyWorldAdapter::default()),
            delay: Box::new(|_| 0),
            response_override: None,
            session_peer_ids: None,
        })
        .await;
        let client = connect_remote_adapter(RemoteAdapterOptions {
            transport: Arc::new(pair.client),
            local_identity: RemoteIdentity {
                seed: seed_client(),
            },
            local_manifest: manifest("test-client", &["spoke-baseline"]),
            remote_pubkey: host_pubkey,
            allowlist: vec![peer_id_host],
            invoke_timeout_ms: None,
            capability_token: None,
        })
        .await
        .expect("dial");
        (client, host)
    }


    fn manifest_with(
        host_id: &str,
        capabilities: &[&str],
        roles: &[&str],
        namespaces: &[&str],
    ) -> spoke_schemas::HostCapabilityManifest {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "host_id": host_id,
            "roles": roles,
            "capabilities": capabilities,
            "namespaces": namespaces,
            "extensions": {},
        }))
        .expect("valid HostCapabilityManifest")
    }

    fn parse_manifest_json(json: &str) -> spoke_schemas::HostCapabilityManifest {
        serde_json::from_str(json).expect("manifest JSON")
    }

    fn orchestrate_upsert_via_ffi(router: &MultiPeerRouterFFI, request: &spoke_schemas::UpsertRequest) {
        for entry in &request.knowledge_entries {
            match router.get_knowledge_entry(entry.entry_id.clone()) {
                Ok(_) => {}
                Err(FfiError::Rejected { code, .. }) if code == "KNOWLEDGE_ENTRY_NOT_FOUND" => {}
                Err(err) => panic!("upsert get via ffi router: {err:?}"),
            }
            let entry_json = serde_json::to_string(entry).expect("entry json");
            router
                .put_knowledge_entry(entry_json, None)
                .expect("upsert put via ffi router");
        }
    }

    #[test]
    fn multi_peer_router_ffi_routes_orchestrate_upsert_equivalent_to_capable_peer() {
        let (baseline_adapter, baseline_host) = ffi_runtime().block_on(async {
            dial_peer([0xa1; 32], manifest("host-baseline", &["spoke-baseline"])).await
        });
        let (computable_adapter, computable_host) = ffi_runtime().block_on(async {
            dial_peer([0xb2; 32], manifest("host-computable", &["l2-computable"])).await
        });

        let router = new_multi_peer_router_ffi();
        let baseline_ffi = RemoteAdapterFFI::from_adapter(baseline_adapter.clone());
        let computable_ffi = RemoteAdapterFFI::from_adapter(computable_adapter.clone());
        router
            .register_peer(baseline_ffi)
            .expect("register baseline");
        router
            .register_peer(computable_ffi)
            .expect("register computable");

        let request = upsert_request(&[fresh_entry("kb_mpr_ffi_upsert", "FFI Router Upsert")]);
        orchestrate_upsert_via_ffi(&router, &request);

        assert_eq!(baseline_host.stats().invokes_dispatched, 2);
        assert_eq!(computable_host.stats().invokes_dispatched, 0);
        assert!(baseline_host
            .inner
            .adapter
            .with_store(|store| store.entries.contains_key("kb_mpr_ffi_upsert")));

        baseline_adapter.close();
        computable_adapter.close();
        baseline_host.close();
        computable_host.close();
    }

    #[test]
    fn multi_peer_router_ffi_rejects_no_capable_peer_for_baseline_ops() {
        let (computable_adapter, computable_host) = ffi_runtime().block_on(async {
            dial_peer([0xb2; 32], manifest("host-computable", &["l2-computable"])).await
        });

        let router = new_multi_peer_router_ffi();
        router
            .register_peer(RemoteAdapterFFI::from_adapter(computable_adapter.clone()))
            .expect("register computable");

        let entry = fresh_entry("kb_mpr_ffi_nomatch", "No Match");
        let err = router
            .put_knowledge_entry(serde_json::to_string(&entry).expect("entry json"), None)
            .expect_err("no capable peer must reject");

        assert!(matches!(
            err,
            FfiError::Rejected {
                code,
                kind: Some(kind),
                wire_code: Some(wire),
                ..
            } if code == "CAPABILITY_PORT_MISSING" && kind == "no_capable_peer" && wire == "no_capable_peer"
        ));
        assert_eq!(computable_host.stats().invokes_dispatched, 0);

        computable_adapter.close();
        computable_host.close();
    }

    #[test]
    fn multi_peer_router_ffi_breaks_ties_on_lowest_peer_id() {
        let (alpha_adapter, alpha_host) = ffi_runtime().block_on(async {
            dial_peer([0xc3; 32], manifest("host-alpha", &["spoke-baseline"])).await
        });
        let (beta_adapter, beta_host) = ffi_runtime().block_on(async {
            dial_peer(
                [0xd4; 32],
                manifest("host-beta", &["spoke-baseline", "l2-computable"]),
            )
            .await
        });
        let (gamma_adapter, gamma_host) = ffi_runtime().block_on(async {
            dial_peer([0xe5; 32], manifest("host-gamma", &["l2-computable"])).await
        });

        let alpha_id = alpha_adapter.remote_peer_id().expect("alpha peer id");
        let beta_id = beta_adapter.remote_peer_id().expect("beta peer id");

        let router = new_multi_peer_router_ffi();
        router
            .register_peer(RemoteAdapterFFI::from_adapter(alpha_adapter.clone()))
            .expect("register alpha");
        router
            .register_peer(RemoteAdapterFFI::from_adapter(beta_adapter.clone()))
            .expect("register beta");
        router
            .register_peer(RemoteAdapterFFI::from_adapter(gamma_adapter.clone()))
            .expect("register gamma");

        let request = upsert_request(&[fresh_entry("kb_mpr_ffi_tiebreak", "Tie-Break")]);
        orchestrate_upsert_via_ffi(&router, &request);

        let (expected_host, other_host) = if alpha_id < beta_id {
            (&alpha_host, &beta_host)
        } else {
            (&beta_host, &alpha_host)
        };
        assert_eq!(expected_host.stats().invokes_dispatched, 2);
        assert_eq!(other_host.stats().invokes_dispatched, 0);
        assert_eq!(gamma_host.stats().invokes_dispatched, 0);

        alpha_adapter.close();
        beta_adapter.close();
        gamma_adapter.close();
        alpha_host.close();
        beta_host.close();
        gamma_host.close();
    }

    #[test]
    fn multi_peer_router_ffi_composes_host_capability_manifest_with_lex_ordered_peers() {
        use serde_json::Value;
        use spoke_schemas::host_capability_manifest::HostCapabilityManifestExtensionsKey;

        let (alpha_adapter, alpha_host) = ffi_runtime().block_on(async {
            dial_peer(
                [0xa1; 32],
                manifest_with(
                    "host-alpha",
                    &["spoke-baseline"],
                    &["data-store"],
                    &["alpha"],
                ),
            )
            .await
        });
        let (beta_adapter, beta_host) = ffi_runtime().block_on(async {
            dial_peer(
                [0xb2; 32],
                manifest_with(
                    "host-beta",
                    &["spoke-baseline", "l2-computable"],
                    &["data-store", "checker"],
                    &["alpha", "beta"],
                ),
            )
            .await
        });

        let alpha_id = alpha_adapter.remote_peer_id().expect("alpha peer id");
        let beta_id = beta_adapter.remote_peer_id().expect("beta peer id");
        let mut expected_peer_order = vec![alpha_id.clone(), beta_id.clone()];
        expected_peer_order.sort();

        let router = new_multi_peer_router_ffi();
        router
            .register_peer(RemoteAdapterFFI::from_adapter(beta_adapter.clone()))
            .expect("register beta");
        router
            .register_peer(RemoteAdapterFFI::from_adapter(alpha_adapter.clone()))
            .expect("register alpha");

        let composed_json = router
            .get_host_capability_manifest()
            .expect("composed manifest via ffi");
        let composed = parse_manifest_json(&composed_json);

        assert_eq!(composed.host_id.as_str(), "multi-peer-router");
        let mut capabilities = composed.capabilities.clone();
        capabilities.sort();
        assert_eq!(capabilities, vec!["l2-computable", "spoke-baseline"]);
        let mut roles = composed.roles.clone();
        roles.sort();
        assert_eq!(roles, vec!["checker", "data-store"]);
        let mut namespaces: Vec<&str> = composed.namespaces.iter().map(|ns| ns.as_str()).collect();
        namespaces.sort();
        assert_eq!(namespaces, vec!["alpha", "beta"]);
        assert!(composed.authority.is_none());

        let router_ext = composed
            .extensions
            .get(&HostCapabilityManifestExtensionsKey::try_from("router").expect("router key"))
            .expect("router extensions");
        let peers = router_ext
            .get("peers")
            .and_then(Value::as_array)
            .expect("peers array");
        let peer_ids: Vec<String> = peers
            .iter()
            .map(|value| value.as_str().expect("peer id string").to_string())
            .collect();
        assert_eq!(peer_ids, expected_peer_order);

        alpha_adapter.close();
        beta_adapter.close();
        alpha_host.close();
        beta_host.close();
    }

    #[test]
    fn multi_peer_router_ffi_lists_peer_manifests_in_lex_peer_id_order() {
        let (alpha_adapter, alpha_host) = ffi_runtime().block_on(async {
            dial_peer(
                [0xa1; 32],
                manifest_with("host-alpha", &["spoke-baseline"], &["data-store"], &["alpha"]),
            )
            .await
        });
        let (beta_adapter, beta_host) = ffi_runtime().block_on(async {
            dial_peer(
                [0xb2; 32],
                manifest_with("host-beta", &["l2-computable"], &["checker"], &["beta"]),
            )
            .await
        });

        let alpha_id = alpha_adapter.remote_peer_id().expect("alpha peer id");
        let beta_id = beta_adapter.remote_peer_id().expect("beta peer id");
        let (first_host_id, second_host_id) = if alpha_id < beta_id {
            ("host-alpha", "host-beta")
        } else {
            ("host-beta", "host-alpha")
        };

        let router = new_multi_peer_router_ffi();
        router
            .register_peer(RemoteAdapterFFI::from_adapter(beta_adapter.clone()))
            .expect("register beta");
        router
            .register_peer(RemoteAdapterFFI::from_adapter(alpha_adapter.clone()))
            .expect("register alpha");

        let manifests_json = router
            .list_peer_host_capability_manifests()
            .expect("per-peer manifests via ffi");
        let manifests: Vec<spoke_schemas::HostCapabilityManifest> =
            serde_json::from_str(&manifests_json).expect("manifest array JSON");
        let host_ids: Vec<&str> = manifests
            .iter()
            .map(|manifest| manifest.host_id.as_str())
            .collect();
        assert_eq!(host_ids, vec![first_host_id, second_host_id]);

        alpha_adapter.close();
        beta_adapter.close();
        alpha_host.close();
        beta_host.close();
    }

    #[test]
    fn multi_peer_router_ffi_empty_router_returns_empty_manifest_views() {
        let router = new_multi_peer_router_ffi();

        let peers_json = router
            .list_peer_host_capability_manifests()
            .expect("empty per-peer list");
        let peers: Vec<spoke_schemas::HostCapabilityManifest> =
            serde_json::from_str(&peers_json).expect("empty manifest array");
        assert!(peers.is_empty());

        let composed = parse_manifest_json(
            &router
                .get_host_capability_manifest()
                .expect("empty composed manifest"),
        );
        assert_eq!(composed.host_id.as_str(), "multi-peer-router");
        assert!(composed.capabilities.is_empty());
        assert!(composed.roles.is_empty());
        assert!(composed.namespaces.is_empty());
        assert!(composed.authority.is_none());
    }

    #[test]
    fn multi_peer_router_ffi_unregister_leaves_adapter_usable() {
        let (adapter, host) = ffi_runtime().block_on(async {
            dial_peer([0xc3; 32], manifest("host-unreg", &["spoke-baseline"])).await
        });
        let ffi_adapter = RemoteAdapterFFI::from_adapter(adapter.clone());
        let peer_id = adapter.remote_peer_id().expect("peer id");

        let router = new_multi_peer_router_ffi();
        router
            .register_peer(ffi_adapter.clone())
            .expect("register peer");
        assert_eq!(router.list_peers(), vec![peer_id.clone()]);

        router.unregister_peer(peer_id.clone());
        assert!(router.list_peers().is_empty());

        assert_eq!(ffi_adapter.state(), "Established");
        let manifest = parse_manifest_json(
            &ffi_adapter
                .get_host_capability_manifest()
                .expect("adapter manifest after unregister"),
        );
        assert_eq!(manifest.host_id.as_str(), "host-unreg");

        let entry = fresh_entry("kb_mpr_ffi_after_unregister", "After Unregister");
        ffi_adapter
            .put_knowledge_entry(serde_json::to_string(&entry).expect("entry json"), None)
            .expect("invoke via adapter after router unregister");
        assert_eq!(host.stats().invokes_dispatched, 1);

        adapter.close();
        host.close();
    }


}

