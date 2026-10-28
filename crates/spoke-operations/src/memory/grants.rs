//! Pure grant-coverage evaluation (§6.5, MEM-R006/R017) and the
//! non-amplification checks for `memory.grants.grant` (§7.9).
//!
//! Providers hold grant state; these helpers are the reusable policy
//! primitives: `grant_covers` answers whether one grant admits one op
//! requirement, and `validate_grant_issuance` answers whether a granter may
//! issue a new grant without amplifying its own coverage.

use crate::result::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use chrono::{DateTime, Utc};
use spoke_schemas::MemoryGrant;
use std::collections::HashSet;

/// What one op needs from a single grant (§6.5 evaluation rules, §7 tables).
#[derive(Debug, Clone)]
pub struct GrantRequirement<'a> {
    /// Required capability from the §7 grant table (`admin` in a grant
    /// implies every capability within that grant's coverage).
    pub capability: &'a str,
    /// Subject the op names; `None` for space-wide ops.
    pub subject_id: Option<&'a str>,
    /// Placement contexts the op names (capture placement, read-by-placement).
    /// Empty = the op names no contexts.
    pub context_ids: Vec<String>,
    /// The op reads shared visibility (`shared_only`, `include_shared`, by-id
    /// shared read, profile read, whole-subject export).
    pub reads_shared: bool,
    /// The op uses a `subject_all` selection — only unrestricted grants pass.
    pub subject_all: bool,
}

struct GrantView<'a> {
    capabilities: &'a [String],
    subject_id: Option<&'a str>,
    /// Empty = unrestricted within the grant's subject/space scope (§6.5).
    context_ids: &'a [String],
    shared_access: bool,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    principal_id: &'a str,
    space_id: &'a str,
}

fn view_of(grant: &MemoryGrant) -> GrantView<'_> {
    match grant {
        MemoryGrant::RestrictedGrant(grant) => GrantView {
            capabilities: &grant.capabilities,
            subject_id: grant.subject_id.as_ref().map(|s| s.as_str()),
            context_ids: &grant.context_ids,
            shared_access: grant.shared_access.unwrap_or(false),
            expires_at: grant.expires_at.as_deref().copied(),
            revoked_at: grant.revoked_at.as_deref().copied(),
            principal_id: grant.principal_id.as_str(),
            space_id: grant.space_id.as_str(),
        },
        MemoryGrant::UnrestrictedGrant(grant) => GrantView {
            capabilities: &grant.capabilities,
            subject_id: grant.subject_id.as_ref().map(|s| s.as_str()),
            context_ids: &[],
            shared_access: false,
            expires_at: grant.expires_at.as_deref().copied(),
            revoked_at: grant.revoked_at.as_deref().copied(),
            principal_id: grant.principal_id.as_str(),
            space_id: grant.space_id.as_str(),
        },
    }
}

/// §6.5 grant evaluation (MEM-R006/R017): is `grant` unexpired, unrevoked,
/// and sufficient for `requirement`?
pub fn grant_covers(
    grant: &MemoryGrant,
    principal_id: &str,
    space_id: &str,
    now: DateTime<Utc>,
    requirement: &GrantRequirement<'_>,
) -> bool {
    let view = view_of(grant);
    if view.principal_id != principal_id || view.space_id != space_id {
        return false;
    }
    if view.revoked_at.is_some() {
        return false;
    }
    if view.expires_at.is_some_and(|expires_at| expires_at <= now) {
        return false;
    }
    if let Some(subject_id) = requirement.subject_id {
        if view.subject_id.is_some_and(|grant_subject| grant_subject != subject_id) {
            return false;
        }
    }
    let has_capability = view.capabilities.iter().any(|capability| {
        capability == requirement.capability || capability == "admin"
    });
    if !has_capability {
        return false;
    }
    let unrestricted = view.context_ids.is_empty();
    if !requirement.context_ids.is_empty()
        && !unrestricted
        && !requirement
            .context_ids
            .iter()
            .all(|context| view.context_ids.contains(context))
    {
        return false;
    }
    if requirement.reads_shared && !unrestricted && !view.shared_access {
        return false;
    }
    if requirement.subject_all && !unrestricted {
        return false;
    }
    true
}

/// The grant being issued via `memory.grants.grant`.
#[derive(Debug, Clone)]
pub struct IssuedGrant<'a> {
    /// Requested capabilities (minItems 1 is type-enforced).
    pub capabilities: &'a [&'a str],
    /// `Some` = subject-scoped issuance; `None` = space-wide.
    pub subject_id: Option<&'a str>,
    /// Requested placement-context restriction; empty = unrestricted.
    pub context_ids: &'a [String],
    /// Requested shared visibility access (meaningful only when restricted).
    pub shared_access: bool,
    /// Requested expiry; `None` = non-expiring.
    pub expires_at: Option<DateTime<Utc>>,
}

fn not_authorized(reason: &str) -> SpokeResult<()> {
    spoke_reject(
        SpokeRejectCode::NotAuthorized,
        format!("Grant issuance would amplify granter coverage: {reason}"),
        None,
    )
}

/// §7.9 non-amplification: every `IssuedGrant` facet must be subsumed by the
/// granter's effective coverage — capabilities, subject scope, context
/// restriction, `shared_access`, and expiry (a granter whose grants expire
/// cannot issue longer-lived or non-expiring grants). All failures reject
/// `NOT_AUTHORIZED`.
pub fn validate_grant_issuance(
    granter_grants: &[MemoryGrant],
    principal_id: &str,
    space_id: &str,
    now: DateTime<Utc>,
    issued: &IssuedGrant<'_>,
) -> SpokeResult<()> {
    // Applicable: valid, and covering the issued subject scope (space-wide
    // granter may issue subject-scoped; subject-scoped granter only the
    // same subject).
    let applicable: Vec<GrantView<'_>> = granter_grants
        .iter()
        .map(view_of)
        .filter(|view| view.principal_id == principal_id && view.space_id == space_id)
        .filter(|view| view.revoked_at.is_none())
        .filter(|view| view.expires_at.is_none_or(|expires_at| expires_at > now))
        .filter(|view| {
            issued
                .subject_id
                .is_none_or(|subject| view.subject_id.is_none() || view.subject_id == Some(subject))
        })
        .collect();
    if applicable.is_empty() {
        return not_authorized("no applicable grant for the issued subject scope");
    }

    let granter_admin = applicable
        .iter()
        .any(|view| view.capabilities.iter().any(|capability| capability == "admin"));
    if !granter_admin {
        let effective: HashSet<&str> = applicable
            .iter()
            .flat_map(|view| view.capabilities.iter().map(String::as_str))
            .collect();
        for capability in issued.capabilities {
            if !effective.contains(capability) {
                return not_authorized("capabilities exceed the granter's effective capabilities");
            }
        }
    }

    if issued.subject_id.is_none()
        && !applicable.iter().any(|view| view.subject_id.is_none())
    {
        return not_authorized("space-wide issuance requires a space-wide granter grant");
    }

    let granter_unrestricted = applicable.iter().any(|view| view.context_ids.is_empty());
    if !issued.context_ids.is_empty() && !granter_unrestricted {
        let granter_contexts: HashSet<&str> = applicable
            .iter()
            .flat_map(|view| view.context_ids.iter().map(String::as_str))
            .collect();
        if !issued
            .context_ids
            .iter()
            .all(|context| granter_contexts.contains(context.as_str()))
        {
            return not_authorized("context_ids exceed the granter's coverage");
        }
    }

    if issued.shared_access
        && !granter_unrestricted
        && !applicable.iter().any(|view| view.shared_access)
    {
        return not_authorized("shared_access requires unrestricted coverage or granter shared_access");
    }

    let earliest_expiry = applicable
        .iter()
        .filter_map(|view| view.expires_at)
        .min();
    match (issued.expires_at, earliest_expiry) {
        (Some(issued_expiry), Some(earliest)) if issued_expiry > earliest => {
            not_authorized("expires_at is later than the granter's earliest applicable expiry")
        }
        (None, Some(_)) => {
            not_authorized("non-expiring issuance requires non-expiring granter coverage")
        }
        _ => spoke_ok_unit(),
    }
}

fn spoke_ok_unit() -> SpokeResult<()> {
    spoke_ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpokeRejectCode, SpokeResult};
    use chrono::{DateTime, Duration, Utc};
    use serde_json::{json, Value};
    use spoke_schemas::MemoryGrant;

    const SPACE: &str = "sp_1";
    const GRANTEE: &str = "p1";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .expect("fixed now")
            .with_timezone(&Utc)
    }

    fn grant_json(extra: Value) -> Value {
        let mut base = json!({
            "schema_version": 1,
            "grant_id": "g1",
            "space_id": SPACE,
            "principal_id": GRANTEE,
            "capabilities": ["capture"],
            "created_at": "2026-01-01T00:00:00Z",
            "revision": 1,
            "extensions": {},
        });
        base.as_object_mut().expect("object").extend(
            extra.as_object().expect("extra object").clone(),
        );
        base
    }

    fn make_grant(extra: Value) -> MemoryGrant {
        serde_json::from_value(grant_json(extra)).expect("valid grant")
    }

    fn requirement(capability: &str) -> GrantRequirement<'_> {
        GrantRequirement {
            capability,
            subject_id: None,
            context_ids: Vec::new(),
            reads_shared: false,
            subject_all: false,
        }
    }

    #[test]
    fn grant_covers_capability_and_admin_implies_all() {
        let grant = make_grant(json!({}));
        assert!(grant_covers(&grant, GRANTEE, SPACE, now(), &requirement("capture")));
        assert!(!grant_covers(&grant, GRANTEE, SPACE, now(), &requirement("recall")));

        let admin = make_grant(json!({"capabilities": ["admin"]}));
        assert!(grant_covers(&admin, GRANTEE, SPACE, now(), &requirement("recall")));
    }

    #[test]
    fn grant_covers_rejects_wrong_principal_space_expired_revoked() {
        let grant = make_grant(json!({}));
        assert!(!grant_covers(&grant, "p_other", SPACE, now(), &requirement("capture")));
        assert!(!grant_covers(&grant, GRANTEE, "sp_other", now(), &requirement("capture")));

        let expired = make_grant(json!({"expires_at": "2026-01-02T00:00:00Z"}));
        assert!(!grant_covers(&expired, GRANTEE, SPACE, now(), &requirement("capture")));

        let revoked = make_grant(json!({"revoked_at": "2026-01-02T00:00:00Z"}));
        assert!(!grant_covers(&revoked, GRANTEE, SPACE, now(), &requirement("capture")));

        let future = make_grant(json!({"expires_at": "2027-01-01T00:00:00Z"}));
        assert!(grant_covers(&future, GRANTEE, SPACE, now(), &requirement("capture")));
    }

    #[test]
    fn grant_covers_subject_matching() {
        let subject_grant = make_grant(json!({"subject_id": "s1"}));
        let req_s1 = GrantRequirement { subject_id: Some("s1"), ..requirement("capture") };
        let req_s2 = GrantRequirement { subject_id: Some("s2"), ..requirement("capture") };
        assert!(grant_covers(&subject_grant, GRANTEE, SPACE, now(), &req_s1));
        assert!(!grant_covers(&subject_grant, GRANTEE, SPACE, now(), &req_s2));

        let space_wide = make_grant(json!({}));
        assert!(grant_covers(&space_wide, GRANTEE, SPACE, now(), &req_s2));
    }

    #[test]
    fn grant_covers_context_coverage() {
        let restricted = make_grant(json!({"context_ids": ["c1"]}));
        let op_c1 = GrantRequirement {
            context_ids: vec!["c1".to_owned()],
            ..requirement("capture")
        };
        let op_c2 = GrantRequirement {
            context_ids: vec!["c2".to_owned()],
            ..requirement("capture")
        };
        assert!(grant_covers(&restricted, GRANTEE, SPACE, now(), &op_c1));
        assert!(!grant_covers(&restricted, GRANTEE, SPACE, now(), &op_c2));

        let unrestricted = make_grant(json!({}));
        assert!(grant_covers(&unrestricted, GRANTEE, SPACE, now(), &op_c2));

        let restricted_two = make_grant(json!({"context_ids": ["c1", "c2"]}));
        let op_both = GrantRequirement {
            context_ids: vec!["c1".to_owned(), "c2".to_owned()],
            ..requirement("capture")
        };
        assert!(grant_covers(&restricted_two, GRANTEE, SPACE, now(), &op_both));
    }

    #[test]
    fn grant_covers_shared_visibility_requires_unrestricted_or_shared_access() {
        let req_shared = GrantRequirement { reads_shared: true, ..requirement("records_read") };

        let restricted_no_shared = make_grant(json!({
            "capabilities": ["records_read"],
            "context_ids": ["c1"],
        }));
        assert!(!grant_covers(&restricted_no_shared, GRANTEE, SPACE, now(), &req_shared));

        let restricted_shared = make_grant(json!({
            "capabilities": ["records_read"],
            "context_ids": ["c1"],
            "shared_access": true,
        }));
        assert!(grant_covers(&restricted_shared, GRANTEE, SPACE, now(), &req_shared));

        let unrestricted = make_grant(json!({"capabilities": ["records_read"]}));
        assert!(grant_covers(&unrestricted, GRANTEE, SPACE, now(), &req_shared));
    }

    #[test]
    fn grant_covers_subject_all_never_passes_context_limited_grants() {
        let req_all = GrantRequirement { subject_all: true, ..requirement("records_read") };

        let restricted = make_grant(json!({
            "capabilities": ["records_read"],
            "context_ids": ["c1"],
            "shared_access": true,
        }));
        assert!(!grant_covers(&restricted, GRANTEE, SPACE, now(), &req_all));

        let unrestricted = make_grant(json!({"capabilities": ["records_read"]}));
        assert!(grant_covers(&unrestricted, GRANTEE, SPACE, now(), &req_all));

        let space_wide_restricted = make_grant(json!({
            "capabilities": ["records_read"],
            "context_ids": ["c1"],
        }));
        assert!(!grant_covers(&space_wide_restricted, GRANTEE, SPACE, now(), &req_all));
    }

    fn issued<'a>(capabilities: &'a [&'a str]) -> IssuedGrant<'a> {
        IssuedGrant {
            capabilities,
            subject_id: None,
            context_ids: &[],
            shared_access: false,
            expires_at: None,
        }
    }

    #[test]
    fn issuance_rejects_capability_amplification() {
        let granter = vec![make_grant(json!({"capabilities": ["capture"]}))];
        let result = validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &issued(&["capture"]));
        assert!(result.is_ok());

        let amplified = issued(&["capture", "admin"]);
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &amplified) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let admin_granter = vec![make_grant(json!({"capabilities": ["admin"]}))];
        assert!(
            validate_grant_issuance(&admin_granter, GRANTEE, SPACE, now(), &issued(&["recall"]))
                .is_ok()
        );
    }

    #[test]
    fn issuance_rejects_subject_scope_amplification() {
        let granter = vec![make_grant(json!({
            "capabilities": ["capture"],
            "subject_id": "s1",
        }))];
        let cross_subject = IssuedGrant { subject_id: Some("s2"), ..issued(&["capture"]) };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &cross_subject) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let same_subject = IssuedGrant { subject_id: Some("s1"), ..issued(&["capture"]) };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &same_subject).is_ok());

        let space_wide_granter = vec![make_grant(json!({"capabilities": ["capture"]}))];
        assert!(
            validate_grant_issuance(&space_wide_granter, GRANTEE, SPACE, now(), &cross_subject)
                .is_ok()
        );
    }

    #[test]
    fn issuance_rejects_context_amplification() {
        let granter = vec![make_grant(json!({
            "capabilities": ["capture"],
            "context_ids": ["c1"],
        }))];
        let wider = IssuedGrant {
            context_ids: &["c1".to_owned(), "c2".to_owned()],
            ..issued(&["capture"])
        };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &wider) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let same = IssuedGrant { context_ids: &["c1".to_owned()], ..issued(&["capture"]) };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &same).is_ok());

        let unrestricted_granter = vec![make_grant(json!({"capabilities": ["capture"]}))];
        assert!(
            validate_grant_issuance(&unrestricted_granter, GRANTEE, SPACE, now(), &wider).is_ok()
        );
    }

    #[test]
    fn issuance_rejects_shared_access_without_granter_shared_access() {
        let granter = vec![make_grant(json!({
            "capabilities": ["records_read"],
            "context_ids": ["c1"],
        }))];
        let shared = IssuedGrant { shared_access: true, ..issued(&["records_read"]) };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &shared) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let granter_with_shared = vec![make_grant(json!({
            "capabilities": ["records_read"],
            "context_ids": ["c1"],
            "shared_access": true,
        }))];
        assert!(
            validate_grant_issuance(&granter_with_shared, GRANTEE, SPACE, now(), &shared).is_ok()
        );
    }

    #[test]
    fn issuance_expiry_cannot_outlive_granter() {
        let granter = vec![make_grant(json!({
            "capabilities": ["capture"],
            "expires_at": "2027-01-01T00:00:00Z",
        }))];

        // Non-expiring issuance while the granter's coverage expires → reject.
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &issued(&["capture"])) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        // Longer-lived issuance → reject.
        let longer: DateTime<Utc> = DateTime::parse_from_rfc3339("2027-06-01T00:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let too_long = IssuedGrant { expires_at: Some(longer), ..issued(&["capture"]) };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &too_long) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        // Shorter-lived issuance → ok.
        let shorter = IssuedGrant {
            expires_at: Some(now() + Duration::days(30)),
            ..issued(&["capture"])
        };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &shorter).is_ok());

        // Non-expiring granter may issue any expiry.
        let eternal_granter = vec![make_grant(json!({"capabilities": ["capture"]}))];
        assert!(
            validate_grant_issuance(&eternal_granter, GRANTEE, SPACE, now(), &too_long).is_ok()
        );
    }

    #[test]
    fn issuance_ignores_other_principals_and_spaces() {
        let other_principal = make_grant(json!({"principal_id": "p9"}));
        let other_space = make_grant(json!({"space_id": "sp_9"}));
        let granter = vec![other_principal, other_space];
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &issued(&["capture"])) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }
    }
}
