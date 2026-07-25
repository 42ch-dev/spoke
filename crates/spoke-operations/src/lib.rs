//! SPOKE lifecycle helpers — pure functions over `spoke-schemas` wire types.

mod assemble;
mod extensions;
mod finding;
mod promote;
mod result;
mod util;

pub use assemble::{
    build_assemble_packet, knowledge_entry_to_assemble_entry, BuildAssemblePacketInput,
};
pub use extensions::{merge_extension_maps, preserve_extension_maps, ExtensionMap};
pub use finding::{is_valid_finding_status_transition, transition_finding_status};
pub use promote::{apply_promote_acceptance, validate_promote_request};
pub use result::{
    spoke_ok, spoke_ok_unit, spoke_reject, SpokeReject, SpokeRejectCode, SpokeResult,
};

/// Re-export wire types for integrator convenience.
pub use spoke_schemas;
