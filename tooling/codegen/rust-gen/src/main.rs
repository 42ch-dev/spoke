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

const EXPECTED_SCHEMA_COUNT: usize = 114;

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
    "data/memory-export-chunk.schema.json",
    "ops/memory-capture-request.schema.json",
    "ops/memory-exports-get-response.schema.json",
    "ops/memory-imports-stage-request.schema.json",
    "ops/memory-records-create-request.schema.json",
    "ops/memory-records-update-request.schema.json",
];

const EMPTY_VEC_ATTR: &str =
    "# [serde (default , skip_serializing_if = \"::std::vec::Vec::is_empty\")] pub ";
const STRUCT_VEC_FIELD: &str = " : :: std :: vec :: Vec <";
const BUILDER_VEC_FIELD: &str = " : :: std :: result :: Result < :: std :: vec :: Vec <";
const BUILDER_STRING_TAIL: &str = " , :: std :: string :: String >";
const SETTER_VEC_BOUND: &str = " < T > (mut self , value : T) -> Self where T : :: std :: convert :: TryInto < :: std :: vec :: Vec <";
const SETTER_VALUE: &str = "value . try_into ()";
const SETTER_SOME: &str = " . map (:: std :: option :: Option :: Some)";

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
type ArrayField = (String, String, String);

/// Array-typed wire fields of the generated module: the optional ones
/// (`serde(default)` + `Vec`) still awaiting presence normalization, and the
/// required arrays that must stay `Vec`.
fn struct_array_fields(rust: &str) -> (Vec<ArrayField>, Vec<ArrayField>) {
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
            if !is_vec_type(type_text) {
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
    let intended_optional: Vec<ArrayField> = intended_all
        .into_iter()
        .filter(|(owner, name, type_text)| matches(owner, name, type_text))
        .collect();
    let required_before: Vec<ArrayField> = required_before
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
        with_builders.push_str(" > > , :: std :: string :: String >");
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
    let leftover_optional: Vec<&ArrayField> = leftover_all
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
    settings.with_struct_builder(true);

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

    let rust = type_space.to_stream().to_string();
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

        match generate_schema_rust(schema_path, &schemas_dir, &out_path, presence_sensitive) {
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
            "{name} : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String > , "
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
        builder.push_str("required : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String > , } ");
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
            "pub mod builder {{ # [derive (Clone , Debug)] pub struct {owner} {{ cognition : :: std :: result :: Result < :: std :: vec :: Vec < super :: CognitionRecord > , :: std :: string :: String > , context_ids : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String > , }} }} "
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
        assert!(output.contains("context_ids : :: std :: result :: Result < :: std :: vec :: Vec < :: std :: string :: String > , :: std :: string :: String >"));
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

    #[test]
    fn scope_refinement_rule_leaves_other_owners_unchanged() {
        // `Rule.entry_types` is an open-vocabulary selector on a non-Scope
        // owner: its generated `Vec` shape must survive this rule.
        let source = module_with_owner("Rule", &[(OPTIONAL_ATTR, "entry_types")]);
        assert_eq!(normalize_scope_arrays(&source).expect("no-op"), source);
    }
}
