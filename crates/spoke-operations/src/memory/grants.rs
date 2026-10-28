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
    // Subject scope (§6.5): a subject-scoped grant never covers another
    // subject, and never covers space-wide ops — a space-wide requirement
    // needs a space-wide grant.
    match (requirement.subject_id, view.subject_id) {
        (Some(op_subject), Some(grant_subject)) if op_subject != grant_subject => return false,
        (None, Some(_)) => return false,
        _ => {}
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
    /// Requested capabilities (§7.9 `minItems: 1`, `uniqueItems: true` —
    /// callers validate; the protocol core vocabulary includes `admin`,
    /// which implies all other capabilities within the grant's coverage).
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

/// §7.9 non-amplification: issuance requires **one** applicable `admin`
/// grant whose coverage subsumes the issued grant's subject scope, context
/// restriction, and `shared_access` together (§6.5 single-grant coverage —
/// coverage from multiple grants is never combined). The expiry bound is
/// global across all applicable admin grants: an issued `expires_at` must
/// be ≤ the earliest applicable admin expiry, and a non-expiring issuance
/// requires that no applicable admin grant expires. All failures reject
/// `NOT_AUTHORIZED`.
pub fn validate_grant_issuance(
    granter_grants: &[MemoryGrant],
    principal_id: &str,
    space_id: &str,
    now: DateTime<Utc>,
    issued: &IssuedGrant<'_>,
) -> SpokeResult<()> {
    // §7.9 Grant: `admin`. Only valid, applicable admin grants can issue.
    let admin_grants: Vec<GrantView<'_>> = granter_grants
        .iter()
        .map(view_of)
        .filter(|view| view.principal_id == principal_id && view.space_id == space_id)
        .filter(|view| view.revoked_at.is_none())
        .filter(|view| view.expires_at.is_none_or(|expires_at| expires_at > now))
        .filter(|view| view.capabilities.iter().any(|capability| capability == "admin"))
        .collect();

    // Expiry bound is computed across ALL applicable admin grants (§7.9: a
    // granter whose grants expire cannot issue longer-lived or non-expiring
    // grants): issued expires_at must be ≤ the earliest applicable admin
    // expiry, and a non-expiring issuance requires that no applicable admin
    // grant expires.
    let earliest_applicable_expiry = admin_grants.iter().filter_map(|view| view.expires_at).min();
    match (issued.expires_at, earliest_applicable_expiry) {
        (Some(issued_expiry), Some(earliest)) if issued_expiry > earliest => {
            return not_authorized(
                "expires_at is later than the granter's earliest applicable admin expiry",
            );
        }
        (None, Some(_)) => {
            return not_authorized(
                "non-expiring issuance requires that no applicable admin grant expires",
            );
        }
        _ => {}
    }

    // One grant must cover every remaining issued facet simultaneously
    // (§6.5 single-grant coverage; disjoint grants are not unioned).
    let covers_issuance = |view: &GrantView<'_>| {
        let subject_ok = match issued.subject_id {
            // Space-wide issuance requires a space-wide admin grant.
            None => view.subject_id.is_none(),
            Some(subject) => view.subject_id.is_none() || view.subject_id == Some(subject),
        };
        let contexts_ok = if issued.context_ids.is_empty() {
            // Unrestricted issuance requires unrestricted-context coverage
            // — a restricted granter can never issue an unrestricted grant.
            view.context_ids.is_empty()
        } else {
            view.context_ids.is_empty()
                || issued
                    .context_ids
                    .iter()
                    .all(|context| view.context_ids.contains(context))
        };
        let shared_ok = !issued.shared_access
            || view.context_ids.is_empty()
            || view.shared_access;
        subject_ok && contexts_ok && shared_ok
    };

    if admin_grants.iter().any(covers_issuance) {
        spoke_ok(())
    } else {
        not_authorized(
            "no single applicable admin grant subsumes the issued grant (subject, contexts, shared_access, expiry)",
        )
    }
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
    fn grant_covers_space_wide_ops_require_space_wide_grants() {
        // §6.5: a subject-scoped grant never covers space-wide operations.
        let subject_grant = make_grant(json!({"subject_id": "s1"}));
        let space_wide_op = requirement("capture");
        assert!(!grant_covers(&subject_grant, GRANTEE, SPACE, now(), &space_wide_op));

        let space_wide = make_grant(json!({}));
        assert!(grant_covers(&space_wide, GRANTEE, SPACE, now(), &space_wide_op));
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
    fn issuance_requires_an_applicable_admin_grant() {
        // §7.9 Grant: `admin`. A capture-only principal can never issue.
        let capture_only = vec![make_grant(json!({"capabilities": ["capture"]}))];
        match validate_grant_issuance(&capture_only, GRANTEE, SPACE, now(), &issued(&["capture"])) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        // An admin grant implies every capability within its coverage.
        let admin_granter = vec![make_grant(json!({"capabilities": ["admin"]}))];
        assert!(
            validate_grant_issuance(&admin_granter, GRANTEE, SPACE, now(), &issued(&["recall"]))
                .is_ok()
        );
    }

    #[test]
    fn issuance_rejects_subject_scope_amplification() {
        let granter = vec![make_grant(json!({
            "capabilities": ["admin"],
            "subject_id": "s1",
        }))];
        let cross_subject = IssuedGrant { subject_id: Some("s2"), ..issued(&["capture"]) };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &cross_subject) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let same_subject = IssuedGrant { subject_id: Some("s1"), ..issued(&["capture"]) };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &same_subject).is_ok());

        let space_wide_granter = vec![make_grant(json!({"capabilities": ["admin"]}))];
        assert!(
            validate_grant_issuance(&space_wide_granter, GRANTEE, SPACE, now(), &cross_subject)
                .is_ok()
        );
    }

    #[test]
    fn issuance_rejects_context_amplification_and_unrestricted_escalation() {
        let granter = vec![make_grant(json!({
            "capabilities": ["admin"],
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

        // Critical: a restricted granter can never issue an unrestricted
        // (contexts omitted) grant — that would escalate to all contexts,
        // shared visibility, and `subject_all`.
        let unrestricted_issuance = issued(&["capture"]);
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &unrestricted_issuance) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let same = IssuedGrant { context_ids: &["c1".to_owned()], ..issued(&["capture"]) };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &same).is_ok());

        let unrestricted_granter = vec![make_grant(json!({"capabilities": ["admin"]}))];
        assert!(
            validate_grant_issuance(&unrestricted_granter, GRANTEE, SPACE, now(), &wider).is_ok()
        );
    }

    #[test]
    fn issuance_never_unions_disjoint_grants() {
        // §6.5/§7.9: one grant must subsume the issuance. Admin for c1 plus
        // a capture grant for c2 cannot jointly authorize a c2 issuance.
        let granter = vec![
            make_grant(json!({"capabilities": ["admin"], "context_ids": ["c1"]})),
            make_grant(json!({"capabilities": ["capture"], "context_ids": ["c2"]})),
        ];
        let issued_c2 = IssuedGrant {
            context_ids: &["c2".to_owned()],
            ..issued(&["capture"])
        };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &issued_c2) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        // Two disjoint admin grants cannot jointly cover a wide issuance.
        let disjoint_admins = vec![
            make_grant(json!({"capabilities": ["admin"], "context_ids": ["c1"]})),
            make_grant(json!({"capabilities": ["admin"], "context_ids": ["c2"]})),
        ];
        let issued_both = IssuedGrant {
            context_ids: &["c1".to_owned(), "c2".to_owned()],
            ..issued(&["capture"])
        };
        match validate_grant_issuance(&disjoint_admins, GRANTEE, SPACE, now(), &issued_both) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }
    }

    #[test]
    fn issuance_rejects_shared_access_without_granter_shared_access() {
        let granter = vec![make_grant(json!({
            "capabilities": ["admin"],
            "context_ids": ["c1"],
        }))];
        let shared = IssuedGrant {
            context_ids: &["c1".to_owned()],
            shared_access: true,
            ..issued(&["records_read"])
        };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &shared) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let granter_with_shared = vec![make_grant(json!({
            "capabilities": ["admin"],
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
            "capabilities": ["admin"],
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
        let eternal_granter = vec![make_grant(json!({"capabilities": ["admin"]}))];
        assert!(
            validate_grant_issuance(&eternal_granter, GRANTEE, SPACE, now(), &too_long).is_ok()
        );
    }

    #[test]
    fn issuance_expiry_is_bounded_by_earliest_applicable_admin_expiry() {
        // Two identical unrestricted admin grants expiring 2027 and 2028:
        // the 2028 expiry does not loosen the bound — the earliest applies.
        let granter = vec![
            make_grant(json!({
                "capabilities": ["admin"],
                "expires_at": "2027-01-01T00:00:00Z",
            })),
            make_grant(json!({
                "capabilities": ["admin"],
                "expires_at": "2028-01-01T00:00:00Z",
            })),
        ];

        let expiry_2028: DateTime<Utc> = DateTime::parse_from_rfc3339("2028-01-01T00:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let at_2028 = IssuedGrant { expires_at: Some(expiry_2028), ..issued(&["capture"]) };
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &at_2028) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }

        let expiry_2027: DateTime<Utc> = DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let at_2027 = IssuedGrant { expires_at: Some(expiry_2027), ..issued(&["capture"]) };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &at_2027).is_ok());

        let earlier = IssuedGrant {
            expires_at: Some(now() + Duration::days(30)),
            ..issued(&["capture"])
        };
        assert!(validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &earlier).is_ok());

        // Non-expiring issuance is rejected while ANY applicable admin
        // grant expires, even though a 2028-expiring grant also covers.
        let non_expiring = issued(&["capture"]);
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &non_expiring) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }
    }

    #[test]
    fn issuance_ignores_other_principals_and_spaces() {
        let other_principal = make_grant(json!({"principal_id": "p9", "capabilities": ["admin"]}));
        let other_space = make_grant(json!({"space_id": "sp_9", "capabilities": ["admin"]}));
        let granter = vec![other_principal, other_space];
        match validate_grant_issuance(&granter, GRANTEE, SPACE, now(), &issued(&["capture"])) {
            SpokeResult::Reject(reject) => assert_eq!(reject.code, SpokeRejectCode::NotAuthorized),
            SpokeResult::Ok(_) => panic!("expected reject"),
        }
    }
}
