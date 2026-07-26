//! SPOKE lifecycle helpers — pure functions over `spoke-schemas` wire types.

mod assemble;
mod body;
mod computable;
mod extensions;
mod error;
mod finding;
mod knowledge_entry;
mod occ;
mod promote;
mod relate;
mod result;
mod scope;
mod upsert;
mod util;

pub use assemble::{
    build_assemble_packet, knowledge_entry_to_assemble_entry, BuildAssemblePacketInput,
    KnowledgeEntryForAssemble,
};
pub use body::{
    filter_body_attributes_by_trait_type, find_body_attribute, list_body_attributes,
    BodyAttributesInput,
};
pub use computable::{
    validate_computable_field_map, validate_computable_log_entry, validate_compute_request,
    validate_compute_request_wire, validate_project_request, validate_project_request_wire,
};
pub use error::{from_error_envelope, to_error_envelope};
pub use extensions::{merge_extension_maps, preserve_extension_maps, ExtensionMap};
pub use finding::{is_valid_finding_status_transition, transition_finding_status};
pub use knowledge_entry::{
    assert_unique_active_knowledge_entry, is_valid_knowledge_entry_status_transition,
    transition_knowledge_entry_status, AssertUniqueActiveKnowledgeEntryInput,
};
pub use occ::assert_revision_match;
pub use promote::{apply_promote_acceptance, validate_promote_request, validate_promote_request_wire};
pub use relate::validate_relate_request;
pub use scope::{
    filter_knowledge_entries_by_scope, filter_knowledge_entries_by_scope_view,
    filter_timeline_events_by_scope, filter_timeline_events_by_scope_view,
    knowledge_entry_matches_scope, knowledge_entry_matches_scope_view, ScopeMatchView,
    timeline_event_matches_scope, timeline_event_matches_scope_view,
};
pub use upsert::{
    validate_upsert_knowledge_entry, UpsertMode, ValidateUpsertKnowledgeEntryContext,
};
pub use result::{
    spoke_ok, spoke_ok_unit, spoke_reject, SpokeReject, SpokeRejectCode, SpokeResult,
};

/// Re-export wire types for integrator convenience.
pub use spoke_schemas;
