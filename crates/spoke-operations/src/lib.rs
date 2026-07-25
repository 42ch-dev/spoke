//! SPOKE lifecycle helpers — pure functions over `spoke-schemas` wire types.

mod assemble;
mod extensions;
mod finding;
mod knowledge_entry;
mod occ;
mod promote;
mod result;
mod util;

pub use assemble::{
    build_assemble_packet, knowledge_entry_to_assemble_entry, BuildAssemblePacketInput,
    KnowledgeEntryForAssemble,
};
pub use extensions::{merge_extension_maps, preserve_extension_maps, ExtensionMap};
pub use finding::{is_valid_finding_status_transition, transition_finding_status};
pub use knowledge_entry::{
    assert_unique_active_knowledge_entry, is_valid_knowledge_entry_status_transition,
    transition_knowledge_entry_status, AssertUniqueActiveKnowledgeEntryInput,
};
pub use occ::assert_revision_match;
pub use promote::{apply_promote_acceptance, validate_promote_request, validate_promote_request_wire};
pub use result::{
    spoke_ok, spoke_ok_unit, spoke_reject, SpokeReject, SpokeRejectCode, SpokeResult,
};

/// Re-export wire types for integrator convenience.
pub use spoke_schemas;
