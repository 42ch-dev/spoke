//! Generate Rust types from SPOKE JSON Schemas using typify.

use glob::glob;
use schemars::schema::RootSchema;
use serde_json::Value;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use typify::{TypeSpace, TypeSpaceSettings};

const EXPECTED_SCHEMA_COUNT: usize = 123;

// typify breaks recursion cycles by boxing the child type id it finds in its
// active set. When the cycle reaches a recursive struct through a *shared*
// `Option<T>` entry (deduped across properties), that entry itself is boxed:
// the struct field becomes `Box<Option<T>>` with a bare `#[serde(default)]`.
// serde then serializes an absent field as `null`, violating the wire rule
// that absent is never serialized as null. Reorder such fields to
// `Option<Box<T>>` with `skip_serializing_if` so the absent member stays
// absent. The remaining occurrences (builder struct fields and setters) use
// the same token prefix and only need the type-order swap; bracket positions
// are unchanged by the swap.
const BARE_DEFAULT_ATTR: &str = "# [serde (default)]";
const BOXED_OPTION: &str = ":: std :: boxed :: Box < :: std :: option :: Option";
const OPTION_BOXED: &str = ":: std :: option :: Option < :: std :: boxed :: Box";
const SKIP_NONE: &str =
    "# [serde (default , skip_serializing_if = \"::std::option::Option::is_none\")]";

// typify maps an optional array to `Vec<T>` + `skip_serializing_if =
// "Vec::is_empty"`, which collapses an explicitly present `[]` into absence on
// the typed round-trip. That is fine for pure value objects, but the memory
// transfer/import carriers are byte bound: §6.13 seals each chunk's JSON minus
// `chunk_digest`, and §7.1 counts the complete supplied `Content`. For those
// modules an optional array becomes `Option<Vec<T>>`, so absence and `[]` stay
// distinct while an explicit `null` stays invalid (the helper below rejects it
// instead of collapsing it to `None`). The builder keeps its `Vec`-taking
// setters and wraps the value in `Some`.
const PRESENCE_SENSITIVE_SCHEMAS: &[&str] = &[
    "data/revision-snapshot.schema.json",
    "data/memory-export-chunk.schema.json",
    "ops/history-revisions-get-response.schema.json",
    "ops/memory-capture-request.schema.json",
    "ops/memory-exports-get-response.schema.json",
    "ops/memory-imports-stage-request.schema.json",
    "ops/memory-records-create-request.schema.json",
    "ops/memory-records-update-request.schema.json",
];

// The same wire-presence rule applies to optional maps (`skip_serializing_if
// = "…Map::is_empty"` collapses `{}` into absence). The history snapshot
// carriers need it because revision replay must keep `modules: {}`,
// `body.state: {}` and friends byte-identical; the adoption request needs it
// because `extensions: {}` is a present member the TypeScript request keeps
// distinct from an absent one (spoke-knowledge-adoption contract §Request
// `extensions` emission). Every optional map in these modules becomes
// `Option<M>` — including enum-variant fields, which carry no `pub` and no
// builder.
const PRESENCE_SENSITIVE_MAP_SCHEMAS: &[&str] = &[
    "data/revision-snapshot.schema.json",
    "ops/history-revisions-get-response.schema.json",
    "ops/knowledge-adopt-request.schema.json",
];

// History replay returns the stored entity byte-for-byte, so the nested
// `created_at` / `updated_at` must keep their wire spelling. typify maps the
// shared `Timestamp` definition to `chrono::DateTime<Utc>`, which re-spells on
// both decode (offset normalized to `Z`) and encode (fractional seconds
// trimmed to 0/3/6/9 digits). These modules' snapshot carriers type those
// members as `StoredTimestamp` — a `String`-backed newtype that still
// validates with chrono on decode (spoke-history §Stored timestamps).
const STORED_TIMESTAMP_SCHEMAS: &[&str] = &[
    "data/revision-snapshot.schema.json",
    "ops/history-revisions-get-response.schema.json",
];

const STORED_TIMESTAMP_FIELDS: [&str; 2] = ["created_at", "updated_at"];
const STORED_TIMESTAMP_OWNER: &str = "RevisionSnapshotSnapshot";
const STORED_TIMESTAMP_OPTION: &str = " : :: std :: option :: Option <";
const STORED_TIMESTAMP_TYPE: &str = " : :: std :: option :: Option < StoredTimestamp >";

const EMPTY_VEC_ATTR: &str =
    "# [serde (default , skip_serializing_if = \"::std::vec::Vec::is_empty\")] pub ";
const STRUCT_VEC_FIELD: &str = " : :: std :: vec :: Vec <";
const BUILDER_VEC_FIELD: &str = " : :: std :: result :: Result < :: std :: vec :: Vec <";
const BUILDER_STRING_TAIL: &str = " , :: std :: string :: String , >";
const SETTER_VEC_BOUND: &str = " < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < :: std :: vec :: Vec <";
const SETTER_VALUE: &str = "value . try_into ()";
const SETTER_SOME: &str = " . map (:: std :: option :: Option :: Some)";

const EMPTY_JSON_MAP_ATTR: &str =
    "# [serde (default , skip_serializing_if = \"::serde_json::Map::is_empty\")] ";
const EMPTY_HASH_MAP_ATTR: &str =
    "# [serde (default , skip_serializing_if = \"::std::collections::HashMap::is_empty\")] ";
const JSON_MAP_TYPE: &str = ":: serde_json :: Map <";
const HASH_MAP_TYPE: &str = ":: std :: collections :: HashMap <";
const BUILDER_RESULT_FIELD: &str = " : :: std :: result :: Result <";
const SETTER_BOUND_PREFIX: &str =
    " < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto <";

// The typify 0.4.x upgrade changed two pieces of the generated public API the
// wire contract already shipped: non-`String` newtypes lost their
// `TryFrom<&str>`/`TryFrom<String>` impls, and several `about` fields that the
// previous typify boxed inside `Option` came back unboxed. Both are breaking
// changes in an additive release, so the generator restores them.

// Schemas whose `about` fields were emitted as `Option<Box<SharedN>>` by the
// previous typify. Box placement was emission-order dependent and cannot be
// derived structurally — `memory-candidate`, `memory-export-chunk` and
// `memory-exports-get-response` carry `about: Option<SharedN>` fields that
// were never boxed and must stay unboxed — so the restore list is explicit.
const ABOUT_BOXED_SCHEMAS: &[&str] = &[
    "data/assemble-packet.schema.json",
    "data/memory-export-bundle.schema.json",
    "data/mind-state.schema.json",
    "ops/assemble-response.schema.json",
    "ops/memory-imports-stage-request.schema.json",
    "ops/memory-recall-response.schema.json",
    "ops/memory-records-get-response.schema.json",
    "ops/memory-records-list-response.schema.json",
];

// Final path segments that are never boxable field targets.
const NEVER_BOXED_ABOUT_TYPES: &[&str] =
    &["String", "Self", "Vec", "Box", "Option", "Result"];

const PRESENCE_ARRAY_HELPER: &str = r#"
/// Presence-sensitive array decode: `Some` keeps an explicitly present array
/// (including `[]`) distinct from an absent member, while an explicit `null`
/// stays invalid — the schema admits arrays only.
fn deserialize_presence_sensitive_array<'de, D, T>(
    deserializer: D,
) -> ::std::result::Result<::std::option::Option<T>, D::Error>
where
    D: ::serde::Deserializer<'de>,
    T: ::serde::Deserialize<'de>,
{
    ::std::result::Result::Ok(::std::option::Option::Some(T::deserialize(
        deserializer,
    )?))
}
"#;

const PRESENCE_MAP_HELPER: &str = r#"
/// Presence-sensitive map decode: `Some` keeps an explicitly present map
/// (including `{}`) distinct from an absent member, while an explicit `null`
/// stays invalid — the schema admits objects only.
fn deserialize_presence_sensitive_map<'de, D, T>(
    deserializer: D,
) -> ::std::result::Result<::std::option::Option<T>, D::Error>
where
    D: ::serde::Deserializer<'de>,
    T: ::serde::Deserialize<'de>,
{
    ::std::result::Result::Ok(::std::option::Option::Some(T::deserialize(
        deserializer,
    )?))
}
"#;

const STORED_TIMESTAMP_IMPL: &str = r#"
/// RFC 3339 timestamp that keeps its stored wire spelling. Decode accepts the
/// same strings as `Timestamp` and encode returns the stored bytes, so a
/// replayed snapshot matches what the provider stored
/// (spoke-history §Stored timestamps).
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
#[serde(try_from = "::std::string::String", into = "::std::string::String")]
pub struct StoredTimestamp(::std::string::String);

impl ::std::convert::TryFrom<&str> for StoredTimestamp {
    type Error = ::chrono::ParseError;

    fn try_from(value: &str) -> ::std::result::Result<Self, Self::Error> {
        crate::timestamp::parse_strict(value)?;
        ::std::result::Result::Ok(Self(value.to_string()))
    }
}

impl ::std::convert::TryFrom<::std::string::String> for StoredTimestamp {
    type Error = ::chrono::ParseError;

    fn try_from(value: ::std::string::String) -> ::std::result::Result<Self, Self::Error> {
        <Self as ::std::convert::TryFrom<&str>>::try_from(value.as_str())
    }
}

impl ::std::convert::From<StoredTimestamp> for ::std::string::String {
    fn from(value: StoredTimestamp) -> Self {
        value.0
    }
}
"#;

// typify maps the shared `Timestamp` definition to `chrono::DateTime<Utc>` and
// lets chrono parse it on both the serde path and `FromStr`; chrono's RFC 3339
// parsers admit a space separator, a `UTC` suffix, un-padded fields and
// surrounding whitespace, all refused by the wire grammar (spoke-data-model
// §Shared wire scalars). Every inlined copy of the definition is the same
// transparent newtype, so the rewrite matches on the inner type instead of the
// type name.
const TRANSPARENT_ATTR: &str = "# [serde (transparent)] pub struct ";
const DATETIME_NEWTYPE_TAIL: &str =
    " (pub :: chrono :: DateTime < :: chrono :: offset :: Utc >) ;";
const STRICT_TIMESTAMP_ATTR: &str =
    "# [serde (try_from = \"::std::string::String\" , into = \"::std::string::String\")] pub struct ";
const CHRONO_PARSE_BODY: &str = "{ Ok (Self (value . parse () ?)) }";
const STRICT_PARSE_BODY: &str = "{ Ok (Self (crate :: timestamp :: parse_strict (value) ?)) }";

/// Strict-timestamp rewrite: every `DateTime<Utc>` newtype gets serde
/// `try_from`/`into = String` and one hand-written strict parser, so decode and
/// `FromStr` agree with the TypeScript gate. Encode uses the spelling chrono's
/// serde impl emits, keeping accepted values byte-identical.
fn rewrite_strict_timestamps(rust: &str) -> Result<String, String> {
    let rule = "strict timestamp";
    let mut out = String::with_capacity(rust.len() + 4096);
    let mut rest = rust;
    let mut names: Vec<String> = Vec::new();

    while let Some(idx) = rest.find(TRANSPARENT_ATTR) {
        out.push_str(&rest[..idx]);
        rest = &rest[idx + TRANSPARENT_ATTR.len()..];
        let name_len = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        let tail = &rest[name_len..];
        if !tail.starts_with(DATETIME_NEWTYPE_TAIL) {
            // Some other transparent newtype keeps its generated attribute.
            out.push_str(TRANSPARENT_ATTR);
            continue;
        }
        out.push_str(STRICT_TIMESTAMP_ATTR);
        out.push_str(name);
        rest = tail;
        names.push(name.to_string());
    }
    out.push_str(rest);

    let datetime_newtypes = rust.matches(DATETIME_NEWTYPE_TAIL).count();
    if datetime_newtypes != names.len() {
        return Err(format!(
            "{rule}: matched {} of {datetime_newtypes} DateTime newtype(s)",
            names.len()
        ));
    }

    for name in &names {
        let impl_head = format!("impl :: std :: str :: FromStr for {name} {{");
        let at = out
            .find(&impl_head)
            .ok_or_else(|| format!("{rule}: no FromStr impl for {name}"))?;
        let body_at = out[at..]
            .find(CHRONO_PARSE_BODY)
            .map(|rel| at + rel)
            .ok_or_else(|| format!("{rule}: unexpected FromStr body for {name}"))?;
        out.replace_range(
            body_at..body_at + CHRONO_PARSE_BODY.len(),
            STRICT_PARSE_BODY,
        );
    }

    if names.is_empty() {
        return Ok(out);
    }
    for name in &names {
        out.push_str(&format!(
            " impl :: std :: convert :: From < {name} > for :: std :: string :: String {{ fn from (value : {name}) -> Self {{ value . 0 . to_rfc3339_opts (:: chrono :: SecondsFormat :: AutoSi , true) }} }}"
        ));
    }
    eprintln!("{rule}s: {} newtype(s)", names.len());
    Ok(out)
}

/// Index of the `>` matching the `<` at `open`.
fn matching_angle(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text[open..].char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Identifier ending exactly at `end`.
fn identifier_before(text: &str, end: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut start = end;
    while start > 0 {
        let byte = bytes[start - 1];
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == end {
        None
    } else {
        Some(text[start..end].to_string())
    }
}

/// Name introduced by the last `keyword` (e.g. `pub struct ` or `impl `).
fn declared_name(text: &str, keyword: &str) -> Option<String> {
    let idx = text.rfind(keyword)? + keyword.len();
    let name: String = text[idx..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Nearest enclosing struct/enum name before a declaration position.
fn enclosing_type(prefix: &str) -> Option<String> {
    match (prefix.rfind("pub struct "), prefix.rfind("pub enum ")) {
        (Some(struct_at), Some(enum_at)) if struct_at > enum_at => {
            declared_name(prefix, "pub struct ")
        }
        (Some(_), Some(_)) => declared_name(prefix, "pub enum "),
        (Some(_), None) => declared_name(prefix, "pub struct "),
        (None, Some(_)) => declared_name(prefix, "pub enum "),
        (None, None) => None,
    }
}

/// Byte index after the string literal that starts at `start` (a quote).
fn skip_string(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Index of the `}` matching the `{` at `open`, skipping string literals.
fn matching_brace(text: &str, open: usize) -> Option<usize> {
    matching_delim(text, open, b'{', b'}')
}

/// Index of the delimiter closing the one at `open`, skipping string literals.
fn matching_delim(text: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            byte if byte == opening => depth += 1,
            byte if byte == closing => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Name and body of every `pub struct Name { .. }` in the generated module.
fn struct_bodies(rust: &str) -> Vec<(&str, &str)> {
    let bytes = rust.as_bytes();
    let mut bodies = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = rust[search_from..].find("pub struct ") {
        let keyword = search_from + rel + "pub struct ".len();
        let mut i = keyword;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let name = &rust[keyword..i];
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if !name.is_empty() && i < bytes.len() && bytes[i] == b'{' {
            if let Some(close) = matching_brace(rust, i) {
                bodies.push((name, &rust[i + 1..close]));
                search_from = close + 1;
                continue;
            }
        }
        search_from = keyword;
    }
    bodies
}

/// Top-level field declaration chunks of one struct body: split on commas at
/// nesting depth zero, skipping string literals.
fn field_chunks(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut chunks = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'<' | b'(' | b'[' | b'{' => depth += 1,
            b'>' | b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                chunks.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    chunks.push(&body[start..]);
    chunks
}

/// Offset of the depth-zero `pub` introducing a struct field, if any. Builder
/// struct fields are private, so only real wire fields match.
fn field_pub_offset(chunk: &str) -> Option<usize> {
    let bytes = chunk.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'<' | b'(' | b'[' | b'{' => depth += 1,
            b'>' | b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'p' if depth == 0
                && chunk[i..].starts_with("pub ")
                && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b']') =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn field_name_and_type(chunk: &str, pub_at: usize) -> Option<(&str, &str)> {
    let after = &chunk[pub_at + "pub ".len()..];
    let name_len = after
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .count();
    if name_len == 0 {
        return None;
    }
    let name = &after[..name_len];
    let rest = after[name_len..].strip_prefix(" : ")?;
    Some((name, rest))
}

/// Index of the first non-whitespace byte at or after `from`.
fn skip_ws(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Argument-list range of the first `serde(...)` attribute at or after `from`.
/// String literals are skipped so doc text cannot be read as an attribute.
fn find_serde_args(text: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if text[i..].starts_with("serde") {
            let open = skip_ws(bytes, i + "serde".len());
            if bytes.get(open) == Some(&b'(') {
                if let Some(close) = matching_delim(text, open, b'(', b')') {
                    return Some((open, close));
                }
            }
            i += "serde".len();
            continue;
        }
        i += 1;
    }
    None
}

/// True when a `serde(...)` argument list carries the `default` argument,
/// whatever its position among the other arguments.
fn serde_args_have_default(args: &str) -> bool {
    let bytes = args.as_bytes();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                if is_default_arg(&args[start..i]) {
                    return true;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    is_default_arg(&args[start..])
}

/// True for the `default` serde argument, including its `= "path"` form.
fn is_default_arg(arg: &str) -> bool {
    let Some(rest) = arg.trim().strip_prefix("default") else {
        return false;
    };
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with('=')
}

/// True for a generated optional-collection attribute carrying `serde(default)`,
/// independent of argument order.
fn attrs_have_serde_default(prefix: &str) -> bool {
    let mut from = 0usize;
    while let Some((open, close)) = find_serde_args(prefix, from) {
        if serde_args_have_default(&prefix[open + 1..close]) {
            return true;
        }
        from = close + 1;
    }
    false
}

fn is_vec_type(type_text: &str) -> bool {
    type_text
        .trim_start()
        .starts_with(":: std :: vec :: Vec <")
}

/// A field selected for a presence rewrite: owning struct, field name and
/// emitted type text.
type WireField = (String, String, String);

fn is_map_type(type_text: &str) -> bool {
    let trimmed = type_text.trim_start();
    trimmed.starts_with(JSON_MAP_TYPE) || trimmed.starts_with(HASH_MAP_TYPE)
}

/// Wire fields of the generated module whose type satisfies `is_target`:
/// the optional ones (`serde(default)`) still awaiting presence
/// normalization, and the required ones that must keep their type. Only
/// `pub` struct fields are seen — enum-variant fields have no `pub` and are
/// covered by the map rule's leftover-attribute check instead.
fn struct_typed_fields(
    rust: &str,
    is_target: fn(&str) -> bool,
) -> (Vec<WireField>, Vec<WireField>) {
    let mut optional = Vec::new();
    let mut required = Vec::new();
    for (owner, body) in struct_bodies(rust) {
        for chunk in field_chunks(body) {
            let Some(pub_at) = field_pub_offset(chunk) else {
                continue;
            };
            let Some((name, type_text)) = field_name_and_type(chunk, pub_at) else {
                continue;
            };
            if !is_target(type_text) {
                continue;
            }
            let entry = (
                owner.to_string(),
                name.to_string(),
                type_text.to_string(),
            );
            if attrs_have_serde_default(&chunk[..pub_at]) {
                optional.push(entry);
            } else {
                required.push(entry);
            }
        }
    }
    (optional, required)
}

fn struct_array_fields(rust: &str) -> (Vec<WireField>, Vec<WireField>) {
    struct_typed_fields(rust, is_vec_type)
}

fn struct_map_fields(rust: &str) -> (Vec<WireField>, Vec<WireField>) {
    struct_typed_fields(rust, is_map_type)
}

/// `Content.cognition` across every inlined clone: typify names the owning
/// struct `Content` in some modules and `SharedNN` in others, so the shared
/// owner/field rule keys on the field name plus its `CognitionRecord` element
/// type instead of a per-module list. Both `content_override` and
/// `fragments[].content` clones match.
fn is_content_cognition(_owner: &str, name: &str, type_text: &str) -> bool {
    name == "cognition" && type_text.contains("CognitionRecord")
}

fn selects_all_fields(_owner: &str, _name: &str, _type_text: &str) -> bool {
    true
}

fn normalize_presence_sensitive_arrays(rust: &str) -> Result<String, String> {
    normalize_arrays_matching(
        rust,
        selects_all_fields,
        "presence-sensitive array",
        true,
    )
}

/// Content.cognition presence rule: only the `cognition` field of an inlined
/// Content clone becomes `Option<Vec<_>>`; every unrelated optional array keeps
/// the generated `Vec` shape.
fn normalize_content_cognition_arrays(rust: &str) -> Result<String, String> {
    normalize_arrays_matching(
        rust,
        is_content_cognition,
        "Content.cognition presence",
        false,
    )
}

/// Scope optional-array refinements: the derived copies appear as `Scope`,
/// `CheckRequestScope` and `AssembleRequestScope`. Omitted is a wildcard while
/// an explicitly present `[]` matches nothing, so presence must survive typed
/// decode on every copy — otherwise the served path widens `[]` to absent.
fn is_scope_refinement_array(owner: &str, name: &str, _type_text: &str) -> bool {
    owner.ends_with("Scope")
        && matches!(name, "entry_ids" | "entry_types" | "timeline_event_ids")
}

/// Scope refinement presence rule (see [`is_scope_refinement_array`]).
fn normalize_scope_arrays(rust: &str) -> Result<String, String> {
    normalize_arrays_matching(
        rust,
        is_scope_refinement_array,
        "Scope refinement presence",
        false,
    )
}

/// Rewrite every `serde(default)` `Vec` wire field selected by `matches` to
/// `Option<Vec<_>>`, together with its builder field and setter. `require_any`
/// fails generation when a module-wide rule selects nothing.
fn normalize_arrays_matching(
    rust: &str,
    matches: fn(&str, &str, &str) -> bool,
    rule: &str,
    require_any: bool,
) -> Result<String, String> {
    // Intended arrays, read from the generator's own emitted shapes: every
    // selected `serde(default)` `Vec` wire field. Required arrays must stay
    // `Vec`, so they are counted separately and re-checked at the end.
    let (intended_all, required_before) = struct_array_fields(rust);
    let intended_optional: Vec<WireField> = intended_all
        .into_iter()
        .filter(|(owner, name, type_text)| matches(owner, name, type_text))
        .collect();
    let required_before: Vec<WireField> = required_before
        .into_iter()
        .filter(|(owner, name, type_text)| matches(owner, name, type_text))
        .collect();
    let mut keys: BTreeSet<(String, String)> = BTreeSet::new();
    let mut out = String::with_capacity(rust.len() + rust.len() / 8);
    let mut rest = rust;

    // Struct and enum-variant fields: `Vec<T>` -> `Option<Vec<T>>`.
    while let Some(idx) = rest.find(EMPTY_VEC_ATTR) {
        let (head, tail) = rest.split_at(idx);
        out.push_str(head);
        let after = &tail[EMPTY_VEC_ATTR.len()..];
        let Some(vec_pos) = after.find(STRUCT_VEC_FIELD) else {
            out.push_str(&tail[..EMPTY_VEC_ATTR.len()]);
            rest = after;
            continue;
        };
        let name = &after[..vec_pos];
        if name.is_empty()
            || !name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        {
            out.push_str(&tail[..EMPTY_VEC_ATTR.len()]);
            rest = after;
            continue;
        }
        let open = vec_pos + STRUCT_VEC_FIELD.len() - 1;
        let close = matching_angle(after, open).ok_or_else(|| {
            format!("{rule}: unbalanced type for field {name}")
        })?;
        let owner = enclosing_type(&out).ok_or_else(|| {
            format!("{rule}: no enclosing type for field {name}")
        })?;
        if !matches(&owner, name, &after[open + 1..close]) {
            out.push_str(&tail[..EMPTY_VEC_ATTR.len()]);
            rest = after;
            continue;
        }
        keys.insert((owner, name.to_string()));
        out.push_str("# [serde (default , deserialize_with = \"deserialize_presence_sensitive_array\" , skip_serializing_if = \"::std::option::Option::is_none\")] pub ");
        out.push_str(name);
        out.push_str(" : :: std :: option :: Option < :: std :: vec :: Vec <");
        out.push_str(&after[open + 1..close]);
        out.push_str(" > >");
        rest = &after[close + 1..];
    }
    out.push_str(rest);

    let intended: BTreeSet<(String, String)> = intended_optional
        .into_iter()
        .map(|(owner, name, _type_text)| (owner, name))
        .collect();
    if keys != intended {
        let missed: Vec<String> = intended
            .difference(&keys)
            .map(|(owner, field)| format!("{owner}::{field}"))
            .collect();
        let unexpected: Vec<String> = keys
            .difference(&intended)
            .map(|(owner, field)| format!("{owner}::{field}"))
            .collect();
        return Err(format!(
            "{rule} normalization coverage mismatch: missed {missed:?}, unexpected {unexpected:?}"
        ));
    }
    if keys.is_empty() {
        if require_any {
            return Err(format!("{rule} normalization matched no fields"));
        }
        return Ok(rust.to_string());
    }

    // Builder struct fields for the same type/field pairs.
    let mut with_builders = String::with_capacity(out.len() + out.len() / 16);
    let mut rest = out.as_str();
    let mut builder_keys: BTreeSet<(String, String)> = BTreeSet::new();
    while let Some(idx) = rest.find(BUILDER_VEC_FIELD) {
        let (head, tail) = rest.split_at(idx);
        with_builders.push_str(head);
        let name = identifier_before(&with_builders, with_builders.len());
        let owner = declared_name(&with_builders, "pub struct ");
        let selected = match (&name, &owner) {
            (Some(field), Some(owner)) => keys.contains(&(owner.clone(), field.clone())),
            _ => false,
        };
        if !selected {
            with_builders.push_str(BUILDER_VEC_FIELD);
            rest = &tail[BUILDER_VEC_FIELD.len()..];
            continue;
        }
        let (Some(field), Some(owner)) = (name, owner) else {
            unreachable!("selected builder field carries a name and owner");
        };
        builder_keys.insert((owner, field.clone()));
        let open = BUILDER_VEC_FIELD.len() - 1;
        let close = matching_angle(tail, open).ok_or_else(|| {
            format!("{rule}: unbalanced builder type for {field}")
        })?;
        let tail_after_vec = close + 1;
        if !tail[tail_after_vec..].starts_with(BUILDER_STRING_TAIL) {
            return Err(format!("{rule}: unexpected builder type tail for {field}"));
        }
        with_builders.push_str(
            " : :: std :: result :: Result < :: std :: option :: Option < :: std :: vec :: Vec <",
        );
        with_builders.push_str(&tail[open + 1..close]);
        with_builders.push_str(" > > , :: std :: string :: String , >");
        rest = &tail[tail_after_vec + BUILDER_STRING_TAIL.len()..];
    }
    with_builders.push_str(rest);

    // Setters: keep the `TryInto<Vec<T>>` argument and wrap it in `Some`.
    let mut with_setters = String::with_capacity(with_builders.len() + 64 * keys.len());
    let mut rest = with_builders.as_str();
    let mut setter_keys: BTreeSet<(String, String)> = BTreeSet::new();
    while let Some(idx) = rest.find(SETTER_VEC_BOUND) {
        let (head, tail) = rest.split_at(idx);
        with_setters.push_str(head);
        let name = identifier_before(&with_setters, with_setters.len());
        let owner = declared_name(&with_setters, "impl ");
        let selected = match (&name, &owner) {
            (Some(name), Some(owner)) => keys.contains(&(owner.clone(), name.clone())),
            _ => false,
        };
        if !selected {
            with_setters.push_str(SETTER_VEC_BOUND);
            rest = &tail[SETTER_VEC_BOUND.len()..];
            continue;
        }
        let (Some(field), Some(owner)) = (name, owner) else {
            unreachable!("selected setter carries a name and owner");
        };
        setter_keys.insert((owner, field));
        let open = SETTER_VEC_BOUND.len() - 1;
        let close = matching_angle(tail, open)
            .ok_or_else(|| format!("{rule}: unbalanced setter bound"))?;
        let body = &tail[close + 1..];
        let Some(value_at) = body.find(SETTER_VALUE) else {
            return Err(format!("{rule}: setter without try_into body"));
        };
        let insert_at = close + 1 + value_at + SETTER_VALUE.len();
        with_setters.push_str(&tail[..insert_at]);
        with_setters.push_str(SETTER_SOME);
        rest = &tail[insert_at..];
    }
    with_setters.push_str(rest);
    // Two presence rules may select fields in the same module; the shared
    // decode helper is emitted once.
    if !with_setters.contains("fn deserialize_presence_sensitive_array") {
        with_setters.push_str(PRESENCE_ARRAY_HELPER);
    }

    // Fail loudly on partial builder/setter drift: every converted wire field
    // must have exactly one builder field and one setter rewrite.
    for (kind, covered) in [("builder", &builder_keys), ("setter", &setter_keys)] {
        if covered != &keys {
            let missed: Vec<String> = keys
                .difference(covered)
                .map(|(owner, field)| format!("{owner}::{field}"))
                .collect();
            return Err(format!(
                "{rule} {kind} coverage mismatch: missed {missed:?}"
            ));
        }
    }

    // Prove the rewrite is complete and that required arrays stayed `Vec`.
    let (leftover_all, required_after_all) = struct_array_fields(&with_setters);
    let leftover_optional: Vec<&WireField> = leftover_all
        .iter()
        .filter(|(owner, name, type_text)| matches(owner, name, type_text))
        .collect();
    if !leftover_optional.is_empty() {
        let missed: Vec<String> = leftover_optional
            .iter()
            .map(|(owner, field, _type_text)| format!("{owner}::{field}"))
            .collect();
        return Err(format!(
            "{rule} normalization left optional Vec fields unconverted: {missed:?}"
        ));
    }
    let required_after = required_after_all
        .iter()
        .filter(|(owner, name, type_text)| matches(owner, name, type_text))
        .count();
    if required_after != required_before.len() {
        return Err(format!(
            "{rule} normalization changed required array count {} -> {}",
            required_before.len(),
            required_after
        ));
    }

    eprintln!(
        "{rule}s: {} field(s), {} builder field(s), {} setter(s), {} required array(s)",
        keys.len(),
        builder_keys.len(),
        setter_keys.len(),
        required_after
    );
    Ok(with_setters)
}

/// Earliest empty-map attribute (`serde_json::Map` or `HashMap`) in `text`.
fn next_empty_map_attr(text: &str) -> Option<(usize, usize)> {
    let json = text.find(EMPTY_JSON_MAP_ATTR);
    let hash = text.find(EMPTY_HASH_MAP_ATTR);
    match (json, hash) {
        (Some(j), Some(h)) if j <= h => Some((j, EMPTY_JSON_MAP_ATTR.len())),
        (Some(_), Some(h)) => Some((h, EMPTY_HASH_MAP_ATTR.len())),
        (Some(j), None) => Some((j, EMPTY_JSON_MAP_ATTR.len())),
        (None, Some(h)) => Some((h, EMPTY_HASH_MAP_ATTR.len())),
        (None, None) => None,
    }
}

/// Presence-sensitive map rewrite for [`PRESENCE_SENSITIVE_MAP_SCHEMAS`]:
/// every `serde(default)` map wire field becomes `Option<M>` so a present `{}`
/// stays distinct from absence, while required maps keep their plain type.
/// Unlike the array rule this also covers enum-variant fields (`modules`,
/// `metadata` on `RevisionSnapshotSnapshot`), which carry no `pub` and no
/// builder; only `pub` struct fields get builder/setter rewrites, tracked in
/// `pub_keys`. Intended coverage for `pub` fields is verified against
/// `struct_map_fields`, and any leftover `Map::is_empty` attribute fails.
fn normalize_presence_sensitive_maps(rust: &str) -> Result<String, String> {
    let rule = "presence-sensitive map";
    let (intended_all, required_before) = struct_map_fields(rust);
    let intended: BTreeSet<(String, String)> = intended_all
        .into_iter()
        .map(|(owner, name, _type_text)| (owner, name))
        .collect();
    let mut keys: BTreeSet<(String, String)> = BTreeSet::new();
    let mut pub_keys: BTreeSet<(String, String)> = BTreeSet::new();
    let mut variant_keys = 0usize;
    let mut out = String::with_capacity(rust.len() + rust.len() / 8);
    let mut rest = rust;

    // Struct and enum-variant fields: `M` -> `Option<M>`; variant fields carry
    // no `pub`, so the prefix is optional here (unlike the array rule).
    while let Some((idx, attr_len)) = next_empty_map_attr(rest) {
        let (head, tail) = rest.split_at(idx);
        out.push_str(head);
        let field = &tail[attr_len..];
        let (had_pub, field) = match field.strip_prefix("pub ") {
            Some(without_pub) => (true, without_pub),
            None => (false, field),
        };
        let name_len: usize = field
            .chars()
            .take_while(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '_')
            .map(char::len_utf8)
            .sum();
        if name_len == 0 {
            return Err(format!("{rule}: attribute not followed by a field name"));
        }
        let name = &field[..name_len];
        let Some(type_part) = field[name_len..].strip_prefix(" : ") else {
            return Err(format!("{rule}: field {name} not followed by ` : `"));
        };
        let type_start = if type_part.starts_with(JSON_MAP_TYPE) {
            JSON_MAP_TYPE.len()
        } else if type_part.starts_with(HASH_MAP_TYPE) {
            HASH_MAP_TYPE.len()
        } else {
            return Err(format!("{rule}: field {name} is not a map type"));
        };
        let open = type_start - 1;
        let close = matching_angle(type_part, open)
            .ok_or_else(|| format!("{rule}: unbalanced type for field {name}"))?;
        let owner = enclosing_type(&out)
            .ok_or_else(|| format!("{rule}: no enclosing type for field {name}"))?;
        keys.insert((owner.clone(), name.to_string()));
        if had_pub {
            pub_keys.insert((owner, name.to_string()));
        } else {
            // Enum-variant fields have no `pub` and no builder; their
            // coverage is the leftover `Map::is_empty` check below.
            variant_keys += 1;
        }
        out.push_str("# [serde (default , deserialize_with = \"deserialize_presence_sensitive_map\" , skip_serializing_if = \"::std::option::Option::is_none\")] ");
        if had_pub {
            out.push_str("pub ");
        }
        out.push_str(name);
        out.push_str(" : :: std :: option :: Option < ");
        out.push_str(&type_part[..=close]);
        out.push_str(" >");
        rest = &type_part[close + 1..];
    }
    out.push_str(rest);

    // `pub` struct fields must match the independent field scan exactly;
    // variant fields are covered by the leftover check below.
    if pub_keys != intended {
        let missed: Vec<String> = intended
            .difference(&pub_keys)
            .map(|(owner, field)| format!("{owner}::{field}"))
            .collect();
        let unexpected: Vec<String> = pub_keys
            .difference(&intended)
            .map(|(owner, field)| format!("{owner}::{field}"))
            .collect();
        return Err(format!(
            "{rule} normalization coverage mismatch: missed {missed:?}, unexpected {unexpected:?}"
        ));
    }
    if keys.is_empty() {
        return Err(format!("{rule} normalization matched no fields"));
    }

    // Builder struct fields for `pub` wire fields:
    // `Result<M, String>` -> `Result<Option<M>, String>`.
    let mut with_builders = String::with_capacity(out.len() + out.len() / 16);
    let mut rest = out.as_str();
    let mut builder_keys: BTreeSet<(String, String)> = BTreeSet::new();
    while let Some(idx) = rest.find(BUILDER_RESULT_FIELD) {
        let (head, tail) = rest.split_at(idx);
        with_builders.push_str(head);
        let inner = &tail[BUILDER_RESULT_FIELD.len()..];
        if !is_map_type(inner) {
            with_builders.push_str(BUILDER_RESULT_FIELD);
            rest = inner;
            continue;
        }
        let name = identifier_before(&with_builders, with_builders.len());
        let owner = declared_name(&with_builders, "pub struct ");
        let selected = match (&name, &owner) {
            (Some(field), Some(owner)) => pub_keys.contains(&(owner.clone(), field.clone())),
            _ => false,
        };
        if !selected {
            with_builders.push_str(BUILDER_RESULT_FIELD);
            rest = inner;
            continue;
        }
        let (Some(field), Some(owner)) = (name, owner) else {
            unreachable!("selected builder field carries a name and owner");
        };
        builder_keys.insert((owner, field.clone()));
        let open = inner
            .find('<')
            .ok_or_else(|| format!("{rule}: builder map type for {field}"))?;
        let close = matching_angle(inner, open).ok_or_else(|| {
            format!("{rule}: unbalanced builder type for {field}")
        })?;
        if !inner[close + 1..].starts_with(BUILDER_STRING_TAIL) {
            return Err(format!("{rule}: unexpected builder type tail for {field}"));
        }
        with_builders.push_str(
            " : :: std :: result :: Result < :: std :: option :: Option < ",
        );
        with_builders.push_str(&inner[..=close]);
        with_builders.push_str(" > , :: std :: string :: String , >");
        rest = &inner[close + 1 + BUILDER_STRING_TAIL.len()..];
    }
    with_builders.push_str(rest);

    // Setters for `pub` fields: keep the `TryInto<M>` argument, wrap `Some`.
    let mut with_setters = String::with_capacity(with_builders.len() + 64 * pub_keys.len());
    let mut rest = with_builders.as_str();
    let mut setter_keys: BTreeSet<(String, String)> = BTreeSet::new();
    while let Some(idx) = rest.find(SETTER_BOUND_PREFIX) {
        let (head, tail) = rest.split_at(idx);
        with_setters.push_str(head);
        let bound = &tail[SETTER_BOUND_PREFIX.len()..];
        if !is_map_type(bound) {
            with_setters.push_str(SETTER_BOUND_PREFIX);
            rest = bound;
            continue;
        }
        let name = identifier_before(&with_setters, with_setters.len());
        let owner = declared_name(&with_setters, "impl ");
        let selected = match (&name, &owner) {
            (Some(name), Some(owner)) => pub_keys.contains(&(owner.clone(), name.clone())),
            _ => false,
        };
        if !selected {
            with_setters.push_str(SETTER_BOUND_PREFIX);
            rest = bound;
            continue;
        }
        let (Some(field), Some(owner)) = (name, owner) else {
            unreachable!("selected setter carries a name and owner");
        };
        setter_keys.insert((owner, field));
        let open = bound
            .find('<')
            .ok_or_else(|| format!("{rule}: unbalanced setter bound"))?;
        let close = matching_angle(bound, open)
            .ok_or_else(|| format!("{rule}: unbalanced setter bound"))?;
        let body = &bound[close + 1..];
        let Some(value_at) = body.find(SETTER_VALUE) else {
            return Err(format!("{rule}: setter without try_into body"));
        };
        let insert_at = SETTER_BOUND_PREFIX.len() + close + 1 + value_at + SETTER_VALUE.len();
        with_setters.push_str(&tail[..insert_at]);
        with_setters.push_str(SETTER_SOME);
        rest = &tail[insert_at..];
    }
    with_setters.push_str(rest);

    if !with_setters.contains("fn deserialize_presence_sensitive_map") {
        with_setters.push_str(PRESENCE_MAP_HELPER);
    }

    // Builder/setter coverage is required for `pub` fields only — enum-variant
    // fields (`modules`, `metadata`) have no builder or setter.
    for (kind, covered) in [("builder", &builder_keys), ("setter", &setter_keys)] {
        if covered != &pub_keys {
            let missed: Vec<String> = pub_keys
                .difference(covered)
                .map(|(owner, field)| format!("{owner}::{field}"))
                .collect();
            return Err(format!(
                "{rule} {kind} coverage mismatch: missed {missed:?}"
            ));
        }
    }

    // Nothing empty-skipping may remain (covers non-canonical attribute
    // spellings on variant fields), and required maps must be untouched.
    if with_setters.contains("Map::is_empty") {
        return Err(format!("{rule} normalization left a Map::is_empty attribute"));
    }
    let (leftover_optional, required_after_all) = struct_map_fields(&with_setters);
    if !leftover_optional.is_empty() {
        let missed: Vec<String> = leftover_optional
            .iter()
            .map(|(owner, field, _type_text)| format!("{owner}::{field}"))
            .collect();
        return Err(format!(
            "{rule} normalization left optional map fields unconverted: {missed:?}"
        ));
    }
    if required_after_all.len() != required_before.len() {
        return Err(format!(
            "{rule} normalization changed required map count {} -> {}",
            required_before.len(),
            required_after_all.len()
        ));
    }

    eprintln!(
        "{rule}s: {} field(s), {} variant field(s), {} builder field(s), {} setter(s), {} required map(s)",
        pub_keys.len(),
        variant_keys,
        builder_keys.len(),
        setter_keys.len(),
        required_after_all.len()
    );
    Ok(with_setters)
}

/// First stored-timestamp wire field in `text`: byte offset and field name.
fn stored_timestamp_field(text: &str) -> Option<(usize, &'static str)> {
    STORED_TIMESTAMP_FIELDS
        .iter()
        .filter_map(|name| {
            let needle = format!("{name} : ");
            text.match_indices(&needle)
                .find(|(at, _)| {
                    let at = *at;
                    at == 0
                        || !(text.as_bytes()[at - 1].is_ascii_alphanumeric()
                            || text.as_bytes()[at - 1] == b'_')
                })
                .map(|(at, _)| (at, *name))
        })
        .min()
}

/// Stored-timestamp rewrite for [`STORED_TIMESTAMP_SCHEMAS`]: every
/// `created_at` / `updated_at` field of `RevisionSnapshotSnapshot` becomes
/// `Option<StoredTimestamp>` so a replay keeps its stored wire spelling.
/// Fields owned by any other type fail generation; the newtype impl is
/// appended once.
fn normalize_stored_timestamps(rust: &str) -> Result<String, String> {
    let rule = "stored timestamp";
    if rust.contains("struct StoredTimestamp") {
        return Err(format!("{rule}: StoredTimestamp already present"));
    }
    let mut out = String::with_capacity(rust.len() + STORED_TIMESTAMP_IMPL.len());
    let mut rest = rust;
    let mut fields = 0usize;
    while let Some((idx, name)) = stored_timestamp_field(rest) {
        out.push_str(&rest[..idx]);
        let field = &rest[idx + name.len()..];
        if !field.starts_with(STORED_TIMESTAMP_OPTION) {
            return Err(format!("{rule}: field {name} is not an optional timestamp"));
        }
        let open = STORED_TIMESTAMP_OPTION.len() - 1;
        let close = matching_angle(field, open)
            .ok_or_else(|| format!("{rule}: unbalanced type for field {name}"))?;
        let owner = enclosing_type(&out)
            .ok_or_else(|| format!("{rule}: no enclosing type for field {name}"))?;
        if owner != STORED_TIMESTAMP_OWNER {
            return Err(format!(
                "{rule}: field {owner}::{name} is outside {STORED_TIMESTAMP_OWNER}"
            ));
        }
        out.push_str(name);
        out.push_str(STORED_TIMESTAMP_TYPE);
        rest = &field[close + 1..];
        fields += 1;
    }
    out.push_str(rest);
    if fields == 0 {
        return Err(format!("{rule} normalization matched no fields"));
    }
    out.push_str(STORED_TIMESTAMP_IMPL);
    eprintln!("{rule}s: {fields} field(s)");
    Ok(out)
}

fn normalize_boxed_option_fields(rust: &str) -> Result<String, String> {
    let mut out = String::with_capacity(rust.len());
    let mut rest = rust;
    let mut swapped_fields = 0usize;

    while let Some(idx) = rest.find(BOXED_OPTION) {
        out.push_str(&rest[..idx]);
        rest = &rest[idx..];

        // Struct field declarations carry the bare default attr immediately
        // before `pub <name> :`. Upgrade that attr so None is not serialized.
        let after_attr = out
            .rfind(BARE_DEFAULT_ATTR)
            .map(|attr_idx| &out[attr_idx + BARE_DEFAULT_ATTR.len()..]);
        if let Some(between) = after_attr {
            let between = between.trim_start_matches(' ');
            if let Some(name) = between.strip_prefix("pub ").and_then(|s| s.strip_suffix(" : "))
            {
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_digit(10) || c == '_') {
                    let attr_idx = out.rfind(BARE_DEFAULT_ATTR).unwrap();
                    out.replace_range(attr_idx..attr_idx + BARE_DEFAULT_ATTR.len(), SKIP_NONE);
                    swapped_fields += 1;
                }
            }
        }

        out.push_str(OPTION_BOXED);
        rest = &rest[BOXED_OPTION.len()..];
    }
    out.push_str(rest);

    if out.contains(BOXED_OPTION) {
        return Err("normalize_boxed_option_fields: leftover Box<Option<..>>".to_string());
    }
    if swapped_fields > 0 {
        eprintln!(
            "normalized {swapped_fields} boxed Option field(s) to Option<Box<..>>"
        );
    }
    Ok(out)
}

/// True when an `Option` inner type is a plain value type that restores to
/// `Box<..>`: a local name (`Shared13`) or `super :: Shared13` path segment.
/// Paths into crates (`:: std ::`, `:: chrono ::`) and primitives/maps stay
/// unboxed, and generic inners (`Vec<T>`, `BTreeMap<..>`) never reach here
/// because the caller only accepts angle-free inners.
fn is_boxable_about_inner(inner: &str) -> bool {
    if inner.contains('<') || inner.contains('"') {
        return false;
    }
    let inner = inner.trim();
    let last = inner.rsplit("::").next().unwrap_or(inner).trim();
    let first = last.chars().next().unwrap_or(' ');
    if !first.is_ascii_uppercase() {
        return false;
    }
    !inner.starts_with("::")
        && last
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !NEVER_BOXED_ABOUT_TYPES.contains(&last)
}

/// Restore `about : Option<SharedN>` -> `about : Option<Box<SharedN>>` for the
/// schemas in [`ABOUT_BOXED_SCHEMAS`], covering the wire field, the builder
/// field and the setter bound. The setter's bound does not follow a `about :`
/// token (`pub fn about < T > ... TryInto < Option < .. > >`), so the bound is
/// located through the setter's own name and rewritten when the enclosing
/// builder already boxed that field.
fn normalize_about_boxed_option_fields(rust: &str) -> Result<String, String> {
    let mut out = String::with_capacity(rust.len() + 512);
    let mut rest = rust;
    let mut keys: BTreeSet<String> = BTreeSet::new();

    // Pass 1: `about : .. Option < inner >` in field/decl positions (wire
    // struct fields and builder struct fields). Everything before `Option`
    // must be whitespace or `Result`-wrapper path tokens.
    while let Some(idx) = rest.find("about") {
        out.push_str(&rest[..idx + "about".len()]);
        rest = &rest[idx + "about".len()..];

        let mut pos = skip_ws(rest.as_bytes(), 0);
        if rest.as_bytes().get(pos) != Some(&b':') {
            continue;
        }
        pos = skip_ws(rest.as_bytes(), pos + 1);

        let window = &rest[pos..(pos + 64).min(rest.len())];
        let Some(option_at) = window.find("Option") else {
            continue;
        };
        if window[..option_at]
            .chars()
            .any(|ch| !(ch == ':' || ch == ' ' || ch == '<' || ch.is_ascii_alphabetic()))
        {
            continue;
        }
        pos = skip_ws(rest.as_bytes(), pos + option_at + "Option".len());
        if rest.as_bytes().get(pos) != Some(&b'<') {
            continue;
        }
        let close =
            matching_angle(rest, pos).ok_or("about-boxing: unbalanced Option type")?;
        let inner = &rest[pos + 1..close];
        if inner.contains("boxed :: Box")
            || inner.contains("::boxed::Box")
            || !is_boxable_about_inner(inner)
        {
            continue;
        }

        if let Some(owner) = enclosing_type(&out) {
            keys.insert(owner);
        }
        out.push_str(&rest[..pos + 1]);
        out.push_str(" :: std :: boxed :: Box < ");
        out.push_str(inner.trim());
        out.push_str(" > >");
        rest = &rest[skip_ws(rest.as_bytes(), close + 1)..];
    }
    out.push_str(rest);

    if keys.is_empty() {
        return Err(
            "about-boxing: schema expected a Box restore but none was emitted".to_string(),
        );
    }

    // Pass 2: `pub fn about` setters on boxed owners. The bound sits inside
    // `TryInto < Option < inner > >` in the signature, so locate `TryInto`
    // before the setter body and wrap the inner when the setter's `impl`
    // owner was boxed in pass 1.
    let mut with_setters = String::with_capacity(out.len() + 128);
    let mut rest = out.as_str();
    let mut setters = 0usize;
    while let Some(idx) = rest.find("pub fn about") {
        with_setters.push_str(&rest[..idx + "pub fn about".len()]);
        rest = &rest[idx + "pub fn about".len()..];

        let body_at = rest.find('{').unwrap_or(rest.len());
        let signature = &rest[..body_at];
        let Some(try_into_at) = signature.find("TryInto") else {
            continue;
        };
        let mut pos = skip_ws(rest.as_bytes(), try_into_at + "TryInto".len());
        if rest.as_bytes().get(pos) != Some(&b'<') {
            continue;
        }
        pos = skip_ws(rest.as_bytes(), pos + 1);
        let window = &rest[pos..(pos + 64).min(rest.len())];
        let Some(option_at) = window.find("Option") else {
            continue;
        };
        if window[..option_at]
            .chars()
            .any(|ch| !(ch == ':' || ch == ' ' || ch.is_ascii_alphabetic()))
        {
            continue;
        }
        pos = skip_ws(rest.as_bytes(), pos + option_at + "Option".len());
        if rest.as_bytes().get(pos) != Some(&b'<') {
            continue;
        }
        let close =
            matching_angle(rest, pos).ok_or("about-boxing: unbalanced setter bound")?;
        let inner = &rest[pos + 1..close];
        if inner.contains("boxed :: Box")
            || inner.contains("::boxed::Box")
            || !is_boxable_about_inner(inner)
        {
            continue;
        }
        match declared_name(&with_setters, "impl ") {
            Some(owner) if keys.contains(&owner) => {}
            _ => continue,
        }

        with_setters.push_str(&rest[..pos + 1]);
        with_setters.push_str(" :: std :: boxed :: Box < ");
        with_setters.push_str(inner.trim());
        with_setters.push_str(" > >");
        rest = &rest[skip_ws(rest.as_bytes(), close + 1)..];
        setters += 1;
    }
    with_setters.push_str(rest);

    eprintln!(
        "restored {} about decl position(s) + {setters} setter bound(s) to Option<Box<..>>",
        keys.len()
    );
    Ok(with_setters)
}

/// Re-emit the `TryFrom<&str>` and `TryFrom<String>` impls typify 0.4.x
/// stopped producing for non-`String` newtypes (`DateTime`, `NonZeroU64`
/// inners such as `Timestamp`, `SchemaVersion` and the shared `SharedN`
/// copies). Detection mirrors the previous typify output: a `FromStr`
/// newtype with a `(pub Inner)` tuple inner and no `TryFrom` impls gets
/// both impls appended; `String` newtypes kept theirs and are skipped.
fn restore_newtype_try_from(rust: &str) -> Result<String, String> {
    let mut inners: Vec<(String, String)> = Vec::new();
    let mut rest = rust;

    while let Some(rel) = rest.find("pub struct ") {
        let name_at = rel + "pub struct ".len();
        let name_len = rest[name_at..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len() - name_at);
        let name = &rest[name_at..name_at + name_len];

        // Tuple newtype shape: `pub struct Name (pub Inner) ;`
        let mut pos = skip_ws(rest.as_bytes(), name_at + name_len);
        if rest.as_bytes().get(pos) == Some(&b'(') {
            pos = skip_ws(rest.as_bytes(), pos + 1);
            if rest[pos..].starts_with("pub") {
                pos = skip_ws(rest.as_bytes(), pos + 3);
                if let Some(close) = rest[pos..].find(')') {
                    let inner = rest[pos..pos + close].trim();
                    if !inner.is_empty() {
                        inners.push((name.to_string(), inner.to_string()));
                    }
                }
            }
        }
        rest = &rest[name_at + name_len..];
    }

    let mut impls = String::new();
    let mut restored = 0usize;
    for (name, inner) in inners {
        // `From<String>`/`From<&str>` activate the blanket TryFrom (U: Into<T>),
        // so a newtype carrying one already has the impl and must not be
        // re-emitted (TimelineScale, whose base output equally lacks explicit
        // TryFrom impls).
        let has_from = |source: &str| {
            rust.contains(&format!(
                "impl :: std :: convert :: From < {source} > for {name} {{"
            ))
        };
        if !rust.contains(&format!("impl :: std :: str :: FromStr for {name} {{"))
            || rust
                .contains(&format!("impl :: std :: convert :: TryFrom < & str > for {name} {{"))
            || rust.contains(&format!(
                "impl :: std :: convert :: TryFrom < String > for {name} {{"
            ))
            || rust.contains(&format!(
                "impl :: std :: convert :: TryFrom < :: std :: string :: String > for {name} {{"
            ))
            || has_from("& str")
            || has_from("String")
            || has_from(":: std :: string :: String")
        {
            continue;
        }
        impls.push_str(&format!(
            " impl :: std :: convert :: TryFrom < & str > for {name} {{ type Error = < {inner} as :: std :: str :: FromStr > :: Err ; fn try_from (value : & str) -> :: std :: result :: Result < Self , Self :: Error > {{ value . parse () }} }}"
        ));
        impls.push_str(&format!(
            " impl :: std :: convert :: TryFrom < String > for {name} {{ type Error = < {inner} as :: std :: str :: FromStr > :: Err ; fn try_from (value : String) -> :: std :: result :: Result < Self , Self :: Error > {{ value . parse () }} }}"
        ));
        restored += 1;
    }

    if restored > 0 {
        eprintln!("restored TryFrom<&str>/TryFrom<String> for {restored} newtype(s)");
        let mut out = String::with_capacity(rust.len() + impls.len());
        out.push_str(rust);
        out.push_str(&impls);
        return Ok(out);
    }
    Ok(rust.to_string())
}


struct GeneratedModule {
    rust_mod: String,
    rel_parent: PathBuf,
    export_all: bool,
    export_type: Option<String>,
}

fn repo_root() -> PathBuf {
    if let Ok(root) = env::var("SPOKE_REPO_ROOT") {
        return PathBuf::from(root);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root")
}

fn schemas_dir(root: &Path) -> PathBuf {
    if let Ok(dir) = env::var("SPOKE_SCHEMAS_DIR") {
        return PathBuf::from(dir);
    }
    root.join("schemas")
}

fn to_rust_module_name(base: &str) -> String {
    base.replace('-', "_")
}

fn source_schemas_dir(root: &Path) -> PathBuf {
    root.join("schemas")
}

fn export_mode_for_schema(schema_path: &Path) -> (bool, Option<String>) {
    let content = fs::read_to_string(schema_path).expect("read source schema");
    let raw: Value = serde_json::from_str(&content).expect("parse source schema");

    let title = raw.get("title").and_then(Value::as_str);

    // Mirror TS buildTypeScriptBarrelExport: single-word title wins over definitions-only
    // (oneOf/anyOf + definitions schemas like upsert-response export the title type only).
    if let Some(t) = title {
        if !t.contains(' ') {
            return (false, Some(t.to_string()));
        }
    }

    let has_definitions = raw.get("definitions").is_some() || raw.get("$defs").is_some();
    let has_object_body = raw.get("type") == Some(&Value::String("object".into()))
        || raw.get("properties").is_some();

    if has_definitions && !has_object_body {
        return (true, None);
    }

    (true, None)
}

fn write_mod_rs(dir: &Path, child_modules: &[String], generated: &[&GeneratedModule]) {
    let mut lines = vec![
        "//! AUTO-GENERATED FROM JSON SCHEMA - DO NOT MODIFY".to_string(),
        "//! Generated by: pnpm run codegen".to_string(),
        String::new(),
    ];

    for m in child_modules {
        lines.push(format!("pub mod {m};"));
    }
    for module in generated {
        lines.push(format!("pub mod {};", module.rust_mod));
    }
    lines.push(String::new());

    for m in child_modules {
        lines.push(format!("pub use {m}::*;"));
    }
    for module in generated {
        if module.export_all {
            lines.push(format!("pub use {}::*;", module.rust_mod));
        } else if let Some(type_name) = &module.export_type {
            lines.push(format!("pub use {}::{type_name};", module.rust_mod));
        }
    }
    lines.push(String::new());

    fs::write(dir.join("mod.rs"), lines.join("\n")).expect("write mod.rs");
}

fn generate_schema_rust(
    schema_path: &Path,
    schemas_dir: &Path,
    out_path: &Path,
    presence_sensitive: bool,
    presence_sensitive_map: bool,
    about_boxed: bool,
    stored_timestamps: bool,
) -> Result<(), String> {
    let content = fs::read_to_string(schema_path)
        .map_err(|err| format!("failed to read {}: {err}", schema_path.display()))?;
    let schema: RootSchema = serde_json::from_str(&content).map_err(|err| {
        format!(
            "invalid JSON Schema in {}: {err}",
            schema_path.display()
        )
    })?;

    let mut settings = TypeSpaceSettings::default();
    settings.map_typespace_settings(|s| s.with_struct_builder(true));

    let mut type_space = TypeSpace::new(&settings);

    let previous_dir = env::current_dir().ok();
    if let Some(parent) = schema_path.parent() {
        let _ = env::set_current_dir(parent);
    }

    let add_result = type_space.add_root_schema(schema);
    if let Some(prev) = previous_dir {
        let _ = env::set_current_dir(prev);
    }

    add_result.map_err(|err| {
        format!(
            "typify failed for {}: {err}",
            schema_path.display()
        )
    })?;

    let rust = type_space
        .to_stream()
        .map_err(|err| format!("typify to_stream failed for {}: {err}", schema_path.display()))?
        .to_string();
    if rust.trim().is_empty() {
        return Err(format!(
            "typify produced empty output for {}",
            schema_path.display()
        ));
    }
    let rust = normalize_boxed_option_fields(&rust).map_err(|err| {
        format!(
            "boxed-option normalization failed for {}: {err}",
            schema_path.display()
        )
    })?;
    // typify 0.4.x stopped emitting TryFrom<&str>/TryFrom<String> for
    // non-String newtypes and unboxed several `about` fields that shipped
    // boxed; restore the released API shape before the other rewrites.
    let rust = restore_newtype_try_from(&rust).map_err(|err| {
        format!(
            "newtype TryFrom restore failed for {}: {err}",
            schema_path.display()
        )
    })?;
    let rust = rewrite_strict_timestamps(&rust).map_err(|err| {
        format!(
            "strict-timestamp rewrite failed for {}: {err}",
            schema_path.display()
        )
    })?;
    let rust = if about_boxed {
        normalize_about_boxed_option_fields(&rust).map_err(|err| {
            format!(
                "about-boxing normalization failed for {}: {err}",
                schema_path.display()
            )
        })?
    } else {
        rust
    };
    let rust = if presence_sensitive {
        normalize_presence_sensitive_arrays(&rust).map_err(|err| {
            format!(
                "presence-sensitive array normalization failed for {}: {err}",
                schema_path.display()
            )
        })?
    } else {
        rust
    };
    let rust = if presence_sensitive_map {
        normalize_presence_sensitive_maps(&rust).map_err(|err| {
            format!(
                "presence-sensitive map normalization failed for {}: {err}",
                schema_path.display()
            )
        })?
    } else {
        rust
    };
    let rust = if stored_timestamps {
        normalize_stored_timestamps(&rust).map_err(|err| {
            format!(
                "stored-timestamp normalization failed for {}: {err}",
                schema_path.display()
            )
        })?
    } else {
        rust
    };
    // Every inlined Content clone keeps absent/`[]`/non-empty distinct,
    // whatever module it was inlined into.
    let rust = normalize_content_cognition_arrays(&rust).map_err(|err| {
        format!(
            "Content.cognition presence normalization failed for {}: {err}",
            schema_path.display()
        )
    })?;

    // Every derived Scope copy keeps optional-array presence distinct, whatever
    // module it was generated into.
    let rust = normalize_scope_arrays(&rust).map_err(|err| {
        format!(
            "Scope refinement presence normalization failed for {}: {err}",
            schema_path.display()
        )
    })?;

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create output directory {}: {err}",
                parent.display()
            )
        })?;
    }

    let header = format!(
        "//! AUTO-GENERATED FROM JSON SCHEMA - DO NOT MODIFY\n//! Source: {}\n//! Generated by: pnpm run codegen\n\n",
        schema_path
            .strip_prefix(schemas_dir)
            .unwrap_or(schema_path)
            .display()
    );
    fs::write(out_path, format!("{header}{rust}")).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            out_path.display()
        )
    })?;
    Ok(())
}

fn main() {
    let root = repo_root();
    let schemas_dir = schemas_dir(&root);
    let source_schemas_dir = source_schemas_dir(&root);
    let out_root = root.join("crates/spoke-schemas/src/generated");

    fs::remove_dir_all(&out_root).ok();
    fs::create_dir_all(&out_root).expect("mkdir out");

    let pattern = schemas_dir.join("**/*.schema.json");
    let mut schema_paths: Vec<PathBuf> = glob(pattern.to_str().unwrap())
        .expect("glob pattern")
        .filter_map(Result::ok)
        .collect();
    schema_paths.sort();

    let mut generated_files: BTreeSet<PathBuf> = BTreeSet::new();
    let mut generated_modules: Vec<GeneratedModule> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for schema_path in &schema_paths {
        let rel = schema_path.strip_prefix(&schemas_dir).expect("under schemas");
        let rel_parent = rel.parent().unwrap_or(Path::new("")).to_path_buf();
        let base = schema_path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("stem")
            .strip_suffix(".schema")
            .unwrap_or("schema");
        let rust_mod = to_rust_module_name(base);
        let out_path = out_root.join(&rel_parent).join(format!("{rust_mod}.rs"));

        let source_schema = source_schemas_dir.join(rel);
        let (export_all, export_type) = export_mode_for_schema(&source_schema);
        let rel_key = rel.to_string_lossy().replace('\\', "/");
        let presence_sensitive = PRESENCE_SENSITIVE_SCHEMAS.contains(&rel_key.as_str());
        let presence_sensitive_map =
            PRESENCE_SENSITIVE_MAP_SCHEMAS.contains(&rel_key.as_str());
        let about_boxed = ABOUT_BOXED_SCHEMAS.contains(&rel_key.as_str());
        let stored_timestamps = STORED_TIMESTAMP_SCHEMAS.contains(&rel_key.as_str());

        match generate_schema_rust(
            schema_path,
            &schemas_dir,
            &out_path,
            presence_sensitive,
            presence_sensitive_map,
            about_boxed,
            stored_timestamps,
        ) {
            Ok(()) => {
                generated_files.insert(out_path.clone());
                generated_modules.push(GeneratedModule {
                    rust_mod: rust_mod.clone(),
                    rel_parent,
                    export_all,
                    export_type,
                });
                println!(
                    "wrote {}",
                    out_path.strip_prefix(&root).unwrap().display()
                );
            }
            Err(err) => failures.push(err),
        }
    }

    if !failures.is_empty() {
        for err in &failures {
            eprintln!("error: {err}");
        }
        process::exit(1);
    }

    if generated_files.len() != EXPECTED_SCHEMA_COUNT {
        eprintln!(
            "error: expected {EXPECTED_SCHEMA_COUNT} Rust schema file(s), generated {}",
            generated_files.len()
        );
        process::exit(1);
    }

    // A typify token change would silently relax every Timestamp newtype; the
    // common module always carries one, so fail generation instead.
    let common_module = out_root.join("common/common.rs");
    let strict_timestamp = fs::read_to_string(&common_module)
        .map(|text| text.contains("crate :: timestamp :: parse_strict (value)"))
        .unwrap_or(false);
    if !strict_timestamp {
        eprintln!(
            "error: no strict Timestamp rewrite in {}",
            common_module.display()
        );
        process::exit(1);
    }

    for sub in ["", "common", "connect", "data", "ops"] {
        let dir = if sub.is_empty() {
            out_root.clone()
        } else {
            out_root.join(sub)
        };
        if !dir.exists() {
            continue;
        }

        let mut child_modules = Vec::new();

        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    child_modules.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }

        child_modules.sort();

        let dir_modules: Vec<&GeneratedModule> = generated_modules
            .iter()
            .filter(|m| {
                let module_dir = if m.rel_parent.as_os_str().is_empty() {
                    out_root.clone()
                } else {
                    out_root.join(&m.rel_parent)
                };
                module_dir == dir
            })
            .collect();

        write_mod_rs(&dir, &child_modules, &dir_modules);
    }

    println!("Rust: {} file(s)", generated_files.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONAL_ATTR: &str =
        "# [serde (default , skip_serializing_if = \"::std::vec::Vec::is_empty\")] pub ";
    const STRING_VEC: &str = ":: std :: vec :: Vec < :: std :: string :: String >";

    fn field(attr: &str, name: &str) -> String {
        format!("{attr}{name} : {STRING_VEC} , ")
    }

    fn builder_field(name: &str) -> String {
        format!(
            "{name} : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String , > , "
        )
    }

    fn setter(name: &str) -> String {
        format!(
            "pub fn {name} < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < :: std :: vec :: Vec < :: std :: string :: String > > , T :: Error : :: std :: fmt :: Display , {{ self . {name} = value . try_into () . map_err (| e | format ! (\"error for {name}: {{e}}\")) ; self }} "
        )
    }

    fn module(fields: &[(&str, &str)]) -> String {
        module_with_owner("Foo", fields)
    }

    fn module_with_owner(owner: &str, fields: &[(&str, &str)]) -> String {
        let mut wire = format!("pub struct {owner} {{ ");
        let mut builder = format!("pub mod builder {{ pub struct {owner} {{ ");
        let mut setter_source = format!("impl {owner} {{ ");
        for (attr, name) in fields {
            wire.push_str(&field(attr, name));
            builder.push_str(&builder_field(name));
            setter_source.push_str(&setter(name));
        }
        wire.push_str("pub required : ");
        wire.push_str(STRING_VEC);
        wire.push_str(" , } ");
        builder.push_str("required : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String , > , } ");
        setter_source.push_str("} }");
        format!("{wire}{builder}{setter_source}")
    }

    #[test]
    fn converts_every_optional_array_and_keeps_required_arrays() {
        let source = module(&[(OPTIONAL_ATTR, "first"), (OPTIONAL_ATTR, "second")]);
        let output = normalize_presence_sensitive_arrays(&source).expect("normalizes");
        assert_eq!(output.matches("deserialize_presence_sensitive_array").count(), 3);
        assert_eq!(
            output.matches("pub first : :: std :: option :: Option < :: std :: vec :: Vec <").count(),
            1
        );
        assert_eq!(
            output.matches("pub second : :: std :: option :: Option < :: std :: vec :: Vec <").count(),
            1
        );
        assert!(
            output.contains("pub required : :: std :: vec :: Vec <"),
            "required arrays stay Vec: {output}"
        );
        assert!(!output.contains("Option < :: std :: vec :: Vec < :: std :: string :: String > > , } "), "required array was wrapped");
    }

    #[test]
    fn partial_attribute_drift_fails_loudly() {
        // The second field still carries an empty-skipping optional Vec but its
        // attribute spelling changed, so the targeted rewrite misses it.
        let drifted = "# [serde (default , skip_serializing_if = \"Vec::is_empty\")] pub ";
        let source = module(&[(OPTIONAL_ATTR, "first"), (drifted, "second")]);
        let error = normalize_presence_sensitive_arrays(&source)
            .expect_err("partial drift must fail generation");
        assert!(
            error.contains("coverage mismatch") && error.contains("second"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn reordered_serde_default_is_still_an_optional_array() {
        // `default` after `skip_serializing_if` is the same serde default, so
        // the field is intended-optional: leaving it as `Vec` must not pass.
        let reordered =
            "# [serde (skip_serializing_if = \"::std::vec::Vec::is_empty\" , default)] pub ";
        let source = module(&[(OPTIONAL_ATTR, "first"), (reordered, "second")]);
        let error = normalize_presence_sensitive_arrays(&source)
            .expect_err("reordered default must not be silently left as Vec");
        assert!(
            error.contains("coverage mismatch") && error.contains("second"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn missing_builder_or_setter_fails_loudly() {
        let source = module(&[(OPTIONAL_ATTR, "first")]);
        let without_setter = source.replace(&setter("first"), "");
        let error = normalize_presence_sensitive_arrays(&without_setter)
            .expect_err("missing setter must fail generation");
        assert!(
            error.contains("setter coverage mismatch") && error.contains("first"),
            "unexpected error: {error}"
        );

        let without_builder = source.replace(&builder_field("first"), "");
        let error = normalize_presence_sensitive_arrays(&without_builder)
            .expect_err("missing builder must fail generation");
        assert!(
            error.contains("builder coverage mismatch") && error.contains("first"),
            "unexpected error: {error}"
        );
    }

    const COGNITION_VEC: &str = ":: std :: vec :: Vec < CognitionRecord >";

    /// Generated shape of one inlined `Content` clone plus an unrelated
    /// optional array, with the matching builder and setters.
    fn content_clone_module(owner: &str) -> String {
        let wire = format!(
            "pub struct {owner} {{ # [serde (default , skip_serializing_if = \"::std::vec::Vec::is_empty\")] pub cognition : {COGNITION_VEC} , # [serde (default , skip_serializing_if = \"::std::vec::Vec::is_empty\")] pub context_ids : {STRING_VEC} , }} "
        );
        let builder = format!(
            "pub mod builder {{ # [derive (Clone , Debug)] pub struct {owner} {{ cognition : :: std :: result :: Result < :: std :: vec :: Vec < super :: CognitionRecord > , :: std :: string :: String , > , context_ids : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String , > , }} }} "
        );
        let setters = format!(
            "impl {owner} {{ pub fn cognition < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < :: std :: vec :: Vec < super :: CognitionRecord > > , T :: Error : :: std :: fmt :: Display , {{ self . cognition = value . try_into () . map_err (| e | format ! (\"error converting supplied value for cognition: {{e}}\")) ; self }} pub fn context_ids < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < :: std :: vec :: Vec < :: std :: string :: String > > , T :: Error : :: std :: fmt :: Display , {{ self . context_ids = value . try_into () . map_err (| e | format ! (\"error converting supplied value for context_ids: {{e}}\")) ; self }} }}"
        );
        format!("{wire}{builder}{setters}")
    }

    #[test]
    fn content_cognition_becomes_presence_sensitive_and_other_arrays_do_not() {
        let source = content_clone_module("Content");
        let output = normalize_content_cognition_arrays(&source).expect("normalizes");
        assert_eq!(output.matches("deserialize_presence_sensitive_array").count(), 2);
        assert!(
            output.contains(
                "# [serde (default , deserialize_with = \"deserialize_presence_sensitive_array\" , skip_serializing_if = \"::std::option::Option::is_none\")] pub cognition : :: std :: option :: Option < :: std :: vec :: Vec < CognitionRecord"
            ),
            "cognition field must keep `[]` distinct: {output}"
        );
        assert!(
            output.contains(
                "# [serde (default , skip_serializing_if = \"::std::vec::Vec::is_empty\")] pub context_ids : :: std :: vec :: Vec < :: std :: string :: String >"
            ),
            "unrelated optional array must keep its Vec shape: {output}"
        );
        assert!(output.contains(
            "cognition : :: std :: result :: Result < :: std :: option :: Option < :: std :: vec :: Vec < super :: CognitionRecord"
        ));
        assert!(output.contains("context_ids : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String , >"));
        assert!(output.contains(
            "self . cognition = value . try_into () . map (:: std :: option :: Option :: Some)"
        ));
        assert!(output.contains("self . context_ids = value . try_into () . map_err"));
    }

    #[test]
    fn content_cognition_rule_covers_shared_owner_names() {
        // typify names an inlined Content clone `SharedNN` in several modules.
        let output = normalize_content_cognition_arrays(&content_clone_module("Shared56"))
            .expect("normalizes");
        assert!(output.contains(
            "pub cognition : :: std :: option :: Option < :: std :: vec :: Vec < CognitionRecord"
        ));
    }

    #[test]
    fn content_cognition_rule_leaves_modules_without_a_content_clone_unchanged() {
        let source = module(&[(OPTIONAL_ATTR, "first")]);
        assert_eq!(
            normalize_content_cognition_arrays(&source).expect("no-op"),
            source
        );
    }

    #[test]
    fn content_cognition_attribute_drift_fails_loudly() {
        let drifted = "# [serde (default , skip_serializing_if = \"Vec::is_empty\")] pub ";
        let source = content_clone_module("Content")
            .replace(OPTIONAL_ATTR, drifted);
        let error = normalize_content_cognition_arrays(&source)
            .expect_err("drifted Content.cognition must fail generation");
        assert!(
            error.contains("coverage mismatch") && error.contains("Content::cognition"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn scope_refinement_arrays_keep_presence_on_every_derived_copy() {
        for owner in ["Scope", "CheckRequestScope", "AssembleRequestScope"] {
            let source = module_with_owner(
                owner,
                &[
                    (OPTIONAL_ATTR, "entry_ids"),
                    (OPTIONAL_ATTR, "entry_types"),
                    (OPTIONAL_ATTR, "timeline_event_ids"),
                ],
            );
            let output = normalize_scope_arrays(&source).expect("normalizes");
            for name in ["entry_ids", "entry_types", "timeline_event_ids"] {
                assert!(
                    output.contains(&format!(
                        "deserialize_with = \"deserialize_presence_sensitive_array\" , skip_serializing_if = \"::std::option::Option::is_none\")] pub {name} : :: std :: option :: Option < :: std :: vec :: Vec <"
                    )),
                    "{owner}::{name} must keep `[]` distinct: {output}"
                );
            }
        }
    }

    const ABOUT_OPT: &str = ":: std :: option :: Option < Shared3 > ";
    const ABOUT_BOXED: &str =
        ":: std :: option :: Option < :: std :: boxed :: Box < Shared3 > >";

    /// Emitted shape of one `about` field set: wire field, builder field and
    /// setter bound, as typify renders them.
    fn about_module() -> String {
        format!(
            "pub struct Target {{ # [serde (skip_serializing_if = \"::std::option::Option::is_none\")] pub about : {ABOUT_OPT} , pub carrier : Shared4 , }} pub mod builder {{ pub struct Target {{ about : :: std :: result :: Result < {ABOUT_OPT} , :: std :: string :: String , > , }} impl Target {{ pub fn about < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < {ABOUT_OPT} > , T :: Error : :: std :: fmt :: Display , {{ self . about = value . try_into () . map_err (| e | format ! (\"error\")) ; self }} }} }}"
        )
    }

    #[test]
    fn about_option_fields_box_wire_builder_and_setter() {
        let output = normalize_about_boxed_option_fields(&about_module()).expect("boxes");
        assert_eq!(output.matches(ABOUT_BOXED).count(), 3, "{output}");
        assert!(
            !output.contains(&format!(" : {ABOUT_OPT} ,")),
            "wire field stayed unboxed"
        );
        assert!(
            !output.contains(&format!("TryInto < {ABOUT_OPT} >")),
            "setter bound stayed unboxed"
        );
    }

    const BOXED_ALREADY: &str =
        ":: std :: option :: Option < :: std :: boxed :: Box < Shared9 > >";

    /// Same shapes as [`about_module`] with one already-boxed self
    /// recursion (`Shared9.about`) plus the unboxed `about` to restore.
    fn about_module_with_self_recursive() -> String {
        format!(
            "pub struct Target {{ pub about : {ABOUT_OPT} , }} pub struct Shared9 {{ pub about : {BOXED_ALREADY} , }} pub mod builder {{ pub struct Shared9 {{ about : :: std :: result :: Result < {BOXED_ALREADY} , :: std :: string :: String , > , }} impl Shared9 {{ pub fn about < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < {BOXED_ALREADY} > , T :: Error : :: std :: fmt :: Display , {{ self }} }} }}"
        )
    }

    #[test]
    fn about_pass_leaves_already_boxed_self_recursion_alone() {
        let output = normalize_about_boxed_option_fields(&about_module_with_self_recursive())
            .expect("boxes the unboxed field");
        assert_eq!(output.matches("boxed :: Box").count(), 4, "{output}");
        assert!(output.contains("pub fn about < T >"));
    }

    #[test]
    fn about_pass_rejects_foreign_and_generic_inners() {
        let source = "pub struct T { pub about : :: std :: option :: Option < :: std :: vec :: Vec < Shared3 > > , }";
        let error = normalize_about_boxed_option_fields(source)
            .expect_err("no boxable inner must fail for a listed schema");
        assert!(error.contains("none was emitted"), "{error}");
    }

    const TIMESTAMP_NEWTYPE: &str =
        "pub struct Timestamp (pub :: chrono :: DateTime < :: chrono :: offset :: Utc >) ; impl :: std :: str :: FromStr for Timestamp { type Err = < :: chrono :: DateTime < :: chrono :: offset :: Utc > as :: std :: str :: FromStr > :: Err ; fn from_str (value : & str) -> :: std :: result :: Result < Self , Self :: Err > { Ok (Self (value . parse () ?)) } }";

    #[test]
    fn newtype_try_from_restores_str_and_string_impls() {
        let output = restore_newtype_try_from(TIMESTAMP_NEWTYPE).expect("restores");
        assert!(
            output.contains("impl :: std :: convert :: TryFrom < & str > for Timestamp {"),
            "{output}"
        );
        assert!(
            output.contains("impl :: std :: convert :: TryFrom < String > for Timestamp {"),
            "{output}"
        );
        assert!(
            output.contains(
                "type Error = < :: chrono :: DateTime < :: chrono :: offset :: Utc > as :: std :: str :: FromStr > :: Err"
            ),
            "{output}"
        );
    }

    #[test]
    fn newtype_try_from_skips_types_that_kept_impls() {
        let source = format!(
            "{TIMESTAMP_NEWTYPE} impl :: std :: convert :: TryFrom < & str > for Timestamp {{ }}"
        );
        assert_eq!(restore_newtype_try_from(&source).expect("no-op"), source);
    }

    #[test]
    fn newtype_try_from_ignores_named_field_structs() {
        let source = "pub struct Truth { pub value : bool , }";
        assert_eq!(restore_newtype_try_from(source).expect("no-op"), source);
    }

    #[test]
    fn scope_refinement_rule_leaves_other_owners_unchanged() {
        // `Rule.entry_types` is an open-vocabulary selector on a non-Scope
        // owner: its generated `Vec` shape must survive this rule.
        let source = module_with_owner("Rule", &[(OPTIONAL_ATTR, "entry_types")]);
        assert_eq!(normalize_scope_arrays(&source).expect("no-op"), source);
    }

    const JSON_MAP_FIELD: &str =
        ":: serde_json :: Map < :: std :: string :: String , :: serde_json :: Value >";

    const HASH_MAP_FIELD: &str = ":: std :: collections :: HashMap < Key , Value >";

    /// One optional `serde_json::Map` struct field, one `HashMap` enum-variant
    /// field, one required `HashMap`, with the builder and setters of the
    /// `pub` field.
    fn map_module() -> String {
        format!(
            "pub enum Snapshot {{ KnowledgeEntry {{ # [serde (default , skip_serializing_if = \"::std::collections::HashMap::is_empty\")] modules : {HASH_MAP_FIELD} , }} }} \
             pub struct Target {{ # [serde (default , skip_serializing_if = \"::serde_json::Map::is_empty\")] pub state : {JSON_MAP_FIELD} , pub required : {HASH_MAP_FIELD} , }} \
             pub mod builder {{ pub struct Target {{ state : :: std :: result :: Result < {JSON_MAP_FIELD} , :: std :: string :: String , > , required : :: std :: result :: Result < {HASH_MAP_FIELD} , :: std :: string :: String , > , }} }} \
             impl Target {{ pub fn state < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < {JSON_MAP_FIELD} > , T :: Error : :: std :: fmt :: Display , {{ self . state = value . try_into () . map_err (| e | format ! (\"error for state: {{e}}\")) ; self }} }}"
        )
    }

    #[test]
    fn map_fields_become_presence_sensitive_and_required_maps_stay() {
        let output = normalize_presence_sensitive_maps(&map_module()).expect("normalizes");
        assert_eq!(output.matches("deserialize_presence_sensitive_map").count(), 3);
        // `pub` struct field keeps `pub`, variant field keeps no `pub`.
        assert!(output.contains(
            "deserialize_with = \"deserialize_presence_sensitive_map\" , skip_serializing_if = \"::std::option::Option::is_none\")] pub state : :: std :: option :: Option <"
        ));
        assert!(output.contains(
            "deserialize_with = \"deserialize_presence_sensitive_map\" , skip_serializing_if = \"::std::option::Option::is_none\")] modules : :: std :: option :: Option <"
        ));
        assert!(
            output.contains(&format!("pub required : {HASH_MAP_FIELD} ,")),
            "required map stays plain: {output}"
        );
        // Builder and setter for the `pub` field wrap `Some`; the variant
        // field has neither.
        assert!(output.contains(&format!(
            "state : :: std :: result :: Result < :: std :: option :: Option <  {JSON_MAP_FIELD} > , :: std :: string :: String , >"
        )), "builder field must wrap Option: {{output}}");
        assert!(output.contains(
            "self . state = value . try_into () . map (:: std :: option :: Option :: Some)"
        ));
        assert!(!output.contains("Map::is_empty"), "leftover empty-map attr: {output}");
    }

    #[test]
    fn map_module_without_optional_maps_fails_loudly() {
        let source = "pub struct Target { pub required : :: std :: collections :: HashMap < Key , Value > , }";
        let error = normalize_presence_sensitive_maps(&source)
            .expect_err("no optional maps must fail generation");
        assert!(error.contains("matched no fields"), "unexpected error: {error}");
    }

    #[test]
    fn missing_map_builder_or_setter_fails_loudly() {
        let source = map_module();
        let setter_start = source.find("pub fn state").expect("setter present");
        let without_setter = format!("{}{}}}", &source[..setter_start], " ");
        let error = normalize_presence_sensitive_maps(&without_setter)
            .expect_err("missing setter must fail generation");
        assert!(
            error.contains("setter coverage mismatch") && error.contains("Target::state"),
            "unexpected error: {error}"
        );

        let without_builder = source.replace(&format!(
            "state : :: std :: result :: Result < {JSON_MAP_FIELD} , :: std :: string :: String , > , "
        ), "");
        let error = normalize_presence_sensitive_maps(&without_builder)
            .expect_err("missing builder must fail generation");
        assert!(
            error.contains("builder coverage mismatch") && error.contains("Target::state"),
            "unexpected error: {error}"
        );
    }

    /// One snapshot carrier with an inline `DateTime<Utc>` field and a shared
    /// `Timestamp` newtype field, both optional.
    const STORED_TIMESTAMP_MODULE: &str = "pub enum RevisionSnapshotSnapshot { KnowledgeEntry { # [serde (skip_serializing_if = \"::std::option::Option::is_none\")] created_at : :: std :: option :: Option < :: chrono :: DateTime < :: chrono :: offset :: Utc > > , updated_at : :: std :: option :: Option < Shared3 > , } }";

    #[test]
    fn snapshot_timestamps_become_stored_timestamps() {
        let output = normalize_stored_timestamps(STORED_TIMESTAMP_MODULE).expect("normalizes");
        let fields = output
            .matches(" : :: std :: option :: Option < StoredTimestamp >")
            .count();
        assert_eq!(fields, 2, "both timestamp fields rewritten: {output}");
        assert!(output.contains("pub struct StoredTimestamp"));
        assert!(
            !output.contains(":: chrono :: DateTime < :: chrono :: offset :: Utc > > ,"),
            "a date-time field was left behind: {output}"
        );
    }

    #[test]
    fn timestamp_fields_outside_the_snapshot_carrier_fail_loudly() {
        let source =
            "pub struct Revision { pub created_at : :: std :: option :: Option < Shared3 > , }";
        let error = normalize_stored_timestamps(source)
            .expect_err("non-snapshot owner must fail generation");
        assert!(
            error.contains("outside RevisionSnapshotSnapshot"),
            "unexpected error: {error}"
        );
    }

    fn datetime_newtype(name: &str) -> String {
        format!(
            "# [serde (transparent)] pub struct {name} (pub :: chrono :: DateTime < :: chrono :: offset :: Utc >) ; impl :: std :: str :: FromStr for {name} {{ type Err = < :: chrono :: DateTime < :: chrono :: offset :: Utc > as :: std :: str :: FromStr > :: Err ; fn from_str (value : & str) -> :: std :: result :: Result < Self , Self :: Err > {{ Ok (Self (value . parse () ?)) }} }}"
        )
    }

    #[test]
    fn datetime_newtypes_parse_through_the_strict_gate() {
        let source = format!(
            "{}{}# [serde (transparent)] pub struct Plain (pub :: std :: string :: String) ;",
            datetime_newtype("Timestamp"),
            datetime_newtype("Shared0"),
        );
        let output = rewrite_strict_timestamps(&source).expect("rewrites");
        assert_eq!(
            output
                .matches("try_from = \"::std::string::String\" , into = \"::std::string::String\"")
                .count(),
            2
        );
        assert_eq!(
            output.matches("crate :: timestamp :: parse_strict (value)").count(),
            2
        );
        assert_eq!(
            output
                .matches("impl :: std :: convert :: From <")
                .count() >= 2,
            true
        );
        assert!(
            output.contains("# [serde (transparent)] pub struct Plain (pub :: std :: string :: String) ;"),
            "unrelated newtype must keep its attribute: {output}"
        );
    }

    #[test]
    fn a_datetime_newtype_without_its_parser_fails_loudly() {
        let broken = datetime_newtype("Timestamp").replace(CHRONO_PARSE_BODY, "");
        let error = rewrite_strict_timestamps(&broken)
            .expect_err("missing FromStr body must fail generation");
        assert!(
            error.contains("unexpected FromStr body for Timestamp"),
            "unexpected error: {error}"
        );
    }
}
