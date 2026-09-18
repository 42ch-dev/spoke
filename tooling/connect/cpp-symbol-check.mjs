#!/usr/bin/env node
/**
 * C ABI header/symbol drift gate.
 *
 * The hand-written `include/spoke_connect.h` and the carrier's exported
 * symbols land together; this gate is what makes that an executable rule
 * instead of a review habit. It performs four checks and exits non-zero if
 * any of them fails:
 *
 *   1. Declaration parse — the marked public declaration block must contain
 *      only prototypes ending in `;` (no conditional declarations, no
 *      unparsed text), no duplicate symbol, and a non-empty set.
 *   2. Export enumeration — every *defined exported* native symbol in the
 *      reserved `spoke_connect_` namespace, read from `nm -gU` (macOS, one
 *      Mach-O leading underscore stripped) or `dumpbin /nologo /exports`
 *      (Windows, export rows only). Missing tool/library, an unparseable row
 *      or an empty export set fails.
 *   3. Two-way diff — declared but not exported, and exported but not
 *      declared, both fail; both lists and the counts are printed.
 *   4. C/C++ declaration probes — a generated C translation unit that takes a
 *      correctly typed, volatile, used function pointer to every declaration
 *      is compiled and linked against the library
 *      (`clang -std=c99 -Wall -Wextra -Werror` / `cl.exe /TC /std:c11 /MD /W4
 *      /WX`), and a C++17 probe with exceptions and RTTI disabled includes the
 *      C header and the convenience header `spoke_connect.hpp` (twice, so
 *      repeated inclusion is covered) and instantiates the convenience value
 *      layer — `Result`, `Buffer`, a move-only handle — so the second header
 *      cannot stop compiling behind a green C-only check.
 *   5. Record layout parity — the carrier reports the size, alignment and
 *      field offsets of every `#[repr(C)]` record it mirrors
 *      (`cargo test -p spoke-connect-capi --lib abi_layout -- --nocapture`)
 *      and a generated translation unit pins each header `sizeof` /
 *      `_Alignof` / `offsetof` to those values
 *      (`clang -std=c11 -Wall -Wextra -Werror` / `cl.exe /TC /std:c11 /W4
 *      /WX`). Both directions are covered: a header record or member the
 *      carrier does not report, and a reported record or member the header
 *      does not declare, both fail — so the record and callback-table block
 *      cannot drift from the mirrors that interpret it.
 *   6. Callback signature parity — the same carrier run reports every callback
 *      typedef and every callback-table member as the Rust type it actually
 *      has, and this gate renders that type into the C signature the header
 *      must declare. Size and offset checks cannot see a callback whose
 *      parameters, return type or calling convention changed, because a
 *      function pointer keeps its size and the table keeps its offsets; a
 *      typed comparison can. Both directions are covered (a typedef or member
 *      on one side only fails), and a member written inline in the header
 *      (the foreign buffer's `release`) is compared the same way as a named
 *      typedef.
 *
 * The compared namespace is `spoke_connect_` only: the carrier links the
 * `spoke-connect` crate, whose UniFFI scaffolding (`uniffi_spoke_connect_*`,
 * `ffi_spoke_connect_*`) is a different ABI and not a consumer entry point.
 *
 * Usage:
 *   node tooling/connect/cpp-symbol-check.mjs --header <header> --library <native>
 *   node tooling/connect/cpp-symbol-check.mjs --header <header> --library <native> --self-test
 *
 * The layout check builds the carrier's Rust test surface, so the gate needs
 * cargo on PATH as well as the C compiler. The repository's local nightly
 * convention (root `AGENTS.md`: the local `-Zno-embed-metadata` flag is
 * nightly-only) is honored automatically; see `cargoCommand`.
 *
 * `--self-test` additionally proves fail-closed behavior: temporary header
 * copies with (a) a real declaration removed, (b) an invented declaration
 * added and (c) a callback argument retyped to a same-size record — which no
 * layout or offset check can see — must all fail their comparison. Temporary
 * files are removed on success and on failure.
 */

import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DECLARATIONS_BEGIN = "/* SPOKE_CONNECT_DECLARATIONS_BEGIN */";
const DECLARATIONS_END = "/* SPOKE_CONNECT_DECLARATIONS_END */";
const SYMBOL_NAMESPACE = "spoke_connect_";
const INVENTED_SYMBOL = "spoke_connect_drift_probe";

/**
 * The carrier's record-layout report lines (`abi_layout.rs`) and the header's
 * record declarations they are compared against. A record is the
 * `typedef struct <Name> { … } <Name>;` form; an opaque handle
 * (`typedef struct <Name> <Name>;`) has no layout and is not one.
 */
const LAYOUT_PREFIX = "SPOKE_CONNECT_ABI_LAYOUT";
/**
 * The carrier's callback signature report lines (`abi_layout.rs`), in the two
 * shapes `SPOKE_CONNECT_ABI_CALLBACK typedef <name> <rust type>` and
 * `SPOKE_CONNECT_ABI_CALLBACK member <record>.<field> <rust type>`.
 */
const CALLBACK_PREFIX = "SPOKE_CONNECT_ABI_CALLBACK";
const LAYOUT_TEST = "abi_layout";
const RECORD_PATTERN =
  /typedef\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{([^{}]*)\}\s*([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
/** The other legal record declaration: an opaque handle with no layout. */
const OPAQUE_PATTERN =
  /typedef\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
const STRUCT_START_PATTERN = /typedef\s+struct\b/g;

/**
 * One callback typedef: `typedef <ret> (SPOKE_CONNECT_CALL *<name>)(<params>)`.
 * The calling-convention macro is part of the grammar, so a typedef that drops
 * it fails the parse instead of passing as a plain function pointer.
 */
const CALLBACK_TYPEDEF_PATTERN = new RegExp(
  "^typedef\\s+([A-Za-z_][A-Za-z0-9_]*)\\s*\\(\\s*SPOKE_CONNECT_CALL\\s*\\*\\s*" +
    "([A-Za-z_][A-Za-z0-9_]*)\\s*\\)\\s*\\(([^()]*)\\)$",
);

/** A callback map member declared inline instead of through a typedef. */
const INLINE_CALLBACK_PATTERN = new RegExp(
  "^(.+?)\\s*\\(\\s*SPOKE_CONNECT_CALL\\s*\\*\\s*" +
    "([A-Za-z_][A-Za-z0-9_]*)\\s*\\)\\s*\\(([^()]*)\\)$",
);

/** The Rust type spellings this ABI's callbacks cross with, as C spellings. */
const RUST_TYPES = new Map([
  ["core::ffi::c_void", "void"],
  ["u8", "uint8_t"],
  ["i32", "int32_t"],
  ["usize", "size_t"],
]);

/**
 * One prototype: `SPOKE_CONNECT_API <int32_t|void> SPOKE_CONNECT_CALL
 * spoke_connect_<name>(<parameter list>)`. Exported parameter lists never
 * contain parentheses, so the grammar stays deliberately strict — a
 * declaration the gate cannot read is a failure, not a silent skip.
 */
const DECLARATION_PATTERN = new RegExp(
  "^SPOKE_CONNECT_API\\s+(int32_t|void)\\s+SPOKE_CONNECT_CALL\\s+" +
    `(${SYMBOL_NAMESPACE}[a-z0-9_]+)\\s*\\(([^()]*)\\)$`,
);

function fail(message) {
  console.error(`cpp-symbol-check: ${message}`);
  process.exit(1);
}

function display(path) {
  const relativePath = relative(REPO_ROOT, path);
  return relativePath.startsWith("..") ? path : relativePath;
}

/**
 * The cargo command prefix: `cargo +nightly` when a nightly toolchain is
 * installed (the repository's local convention — root `AGENTS.md`), plain
 * `cargo` otherwise (CI, which has no nightly and pins the stable toolchain).
 */
function cargoCommand() {
  const rustup = spawnSync("rustup", ["toolchain", "list"], { encoding: "utf8" });
  if (!rustup.error && rustup.status === 0 && /^nightly/m.test(rustup.stdout ?? "")) {
    return ["cargo", "+nightly"];
  }
  return ["cargo"];
}

function parseArgs(argv) {
  const args = { header: null, library: null, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--header") {
      args.header = argv[index + 1];
      index += 1;
    } else if (flag === "--library") {
      args.library = argv[index + 1];
      index += 1;
    } else if (flag === "--self-test") {
      args.selfTest = true;
    } else {
      fail(
        `unknown argument '${flag}' ` +
          "(usage: --header <header> --library <native> [--self-test])",
      );
    }
  }
  if (!args.header) fail("--header is required");
  if (!args.library) fail("--library is required");
  args.header = resolve(REPO_ROOT, args.header);
  args.library = resolve(REPO_ROOT, args.library);
  return args;
}

function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, " ").replace(/\/\/[^\n]*/g, " ");
}

/** Parses the marked declaration block into `{name, returnType, params}`. */
function parseDeclarations(headerText, headerPath) {
  const begin = headerText.indexOf(DECLARATIONS_BEGIN);
  const end = headerText.indexOf(DECLARATIONS_END);
  if (begin < 0 || end < 0) {
    fail(
      `${display(headerPath)} is missing the ${DECLARATIONS_BEGIN} / ` +
        `${DECLARATIONS_END} markers`,
    );
  }
  if (end < begin) fail(`${display(headerPath)}: declaration markers are inverted`);

  const body = stripComments(
    headerText.slice(begin + DECLARATIONS_BEGIN.length, end),
  );
  const declarations = [];
  const unparsed = [];
  for (const chunk of body.split(";")) {
    const candidate = chunk.replace(/\s+/g, " ").trim();
    if (!candidate) continue;
    const match = DECLARATION_PATTERN.exec(candidate);
    if (!match) {
      unparsed.push(candidate);
      continue;
    }
    declarations.push({ name: match[2], returnType: match[1], params: match[3] });
  }

  if (unparsed.length > 0) {
    fail(
      `${display(headerPath)}: ${unparsed.length} unparsed declaration block ` +
        `entr${unparsed.length === 1 ? "y" : "ies"} (conditional declarations and ` +
        `non-prototype text are not allowed):\n` +
        unparsed.map((entry) => `  ${entry.slice(0, 160)}`).join("\n"),
    );
  }
  if (declarations.length === 0) {
    fail(`${display(headerPath)}: the declaration block is empty`);
  }
  const seen = new Set();
  for (const declaration of declarations) {
    if (seen.has(declaration.name)) {
      fail(`${display(headerPath)}: duplicate declaration '${declaration.name}'`);
    }
    seen.add(declaration.name);
  }
  return declarations;
}

/**
 * Parses the header's record declarations into `{name, fields}` in declaration
 * order. Members name the pointer for a callback function pointer
 * (`void (SPOKE_CONNECT_CALL *release)(…)`, calling convention included) and
 * the trailing identifier otherwise (`const uint8_t *data`). Opaque handle
 * declarations (`typedef struct <Name> <Name>;`) carry no layout and are
 * skipped; any other `typedef struct` shape fails, so a record the parser
 * cannot read is never left silently unverified.
 */
function parseRecords(headerText, headerPath) {
  const body = stripComments(headerText);
  const records = [];
  const covered = new Set();
  for (const match of body.matchAll(RECORD_PATTERN)) {
    covered.add(match.index);
    const [, tag, members, alias] = match;
    if (tag !== alias) {
      fail(
        `${display(headerPath)}: record '${tag}' is aliased as '${alias}'; ` +
          "the layout check compares records by name",
      );
    }
    const fields = [];
    for (const chunk of members.split(";")) {
      const member = chunk.trim().replace(/\s+/g, " ");
      if (!member) continue;
      const pointer = /\(\s*(?:[A-Za-z_][A-Za-z0-9_]*\s+)*\*\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)/.exec(
        member,
      );
      const name = pointer
        ? pointer[1]
        : (/([A-Za-z_][A-Za-z0-9_]*)$/.exec(member) ?? [])[1];
      if (!name) fail(`${display(headerPath)}: unparsed '${tag}' member: '${member}'`);
      // The declaration minus the declarator name: the member's type, which is
      // a callback typedef when it names one (an inline callback declarator is
      // read from `text` instead, since its type is split around the name).
      fields.push({ name, type: member.slice(0, member.lastIndexOf(name)).trim(), text: member });
    }
    if (fields.length === 0) {
      fail(`${display(headerPath)}: record '${tag}' declares no member`);
    }
    records.push({ name: tag, fields });
  }

  if (records.length === 0) {
    fail(`${display(headerPath)}: no record declarations found`);
  }
  for (const match of body.matchAll(OPAQUE_PATTERN)) {
    covered.add(match.index);
  }
  const unparsed = [];
  for (const match of body.matchAll(STRUCT_START_PATTERN)) {
    if (!covered.has(match.index)) {
      unparsed.push(body.slice(match.index, match.index + 80).replace(/\s+/g, " ").trim());
    }
  }
  if (unparsed.length > 0) {
    fail(
      `${display(headerPath)}: ${unparsed.length} unparsed record declaration` +
        `${unparsed.length === 1 ? "" : "s"} (only 'typedef struct <Name> { … } ` +
        `<Name>;' and the opaque 'typedef struct <Name> <Name>;' forms carry a ` +
        `layout the gate can compare):\n` +
        unparsed.map((entry) => `  ${entry}`).join("\n"),
    );
  }
  const seen = new Set();
  for (const record of records) {
    if (seen.has(record.name)) {
      fail(`${display(headerPath)}: duplicate record '${record.name}'`);
    }
    seen.add(record.name);
  }
  return records;
}

/**
 * Runs the carrier's report (`crates/spoke-connect-capi/src/abi_layout.rs`) and
 * parses it into the record layouts (`name → {size, align, fields}`), the
 * callback typedefs (name → Rust type) and the callback members (label → Rust
 * type). An empty report of either kind fails: without it that comparison
 * would pass vacuously.
 */
function carrierReport() {
  const [command, ...prefix] = cargoCommand();
  const args = [
    ...prefix,
    "test",
    "--locked",
    "-p",
    "spoke-connect-capi",
    "--lib",
    LAYOUT_TEST,
    "--",
    "--nocapture",
  ];
  const result = spawnSync(command, args, { cwd: REPO_ROOT, encoding: "utf8" });
  if (result.error) {
    fail(`failed to run '${command}': ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(
      `'${command} ${args.join(" ")}' exited ${result.status}\n` +
        `${(result.stdout ?? "").trim()}\n${(result.stderr ?? "").trim()}`,
    );
  }

  const records = new Map();
  const typedefs = new Map();
  const members = new Map();
  for (const line of `${result.stdout ?? ""}\n${result.stderr ?? ""}`.split("\n")) {
    if (line.startsWith(`${CALLBACK_PREFIX} `)) {
      const row = /^(\S+) (\S+) (.+)$/.exec(line.slice(CALLBACK_PREFIX.length + 1).trim());
      if (!row) fail(`unparsed '${CALLBACK_PREFIX}' row: '${line.trim()}'`);
      const [, kind, label, rustType] = row;
      const target = { typedef: typedefs, member: members }[kind];
      if (!target) fail(`unknown '${CALLBACK_PREFIX}' kind '${kind}' in '${line.trim()}'`);
      if (target.has(label)) fail(`duplicate '${CALLBACK_PREFIX}' ${kind} '${label}'`);
      target.set(label, rustType);
      continue;
    }
    if (!line.startsWith(`${LAYOUT_PREFIX} `)) continue;
    const [name, ...pairs] = line.slice(LAYOUT_PREFIX.length + 1).trim().split(/\s+/);
    const record = { size: null, align: null, fields: new Map() };
    for (const pair of pairs) {
      const [key, value] = pair.split("=");
      if (!/^[a-z0-9_]+$/.test(key ?? "") || !/^\d+$/.test(value ?? "")) {
        fail(`unparsed '${LAYOUT_PREFIX}' row: '${line.trim()}'`);
      }
      if (key === "size") record.size = Number(value);
      else if (key === "align") record.align = Number(value);
      else record.fields.set(key, Number(value));
    }
    if (record.size === null || record.align === null) {
      fail(`'${LAYOUT_PREFIX}' row '${name}' reports no size or alignment`);
    }
    if (records.has(name)) fail(`duplicate '${LAYOUT_PREFIX}' row for '${name}'`);
    records.set(name, record);
  }

  if (records.size === 0) {
    fail(
      `the carrier reported no '${LAYOUT_PREFIX}' records ` +
        "(is the carrier test surface built?)",
    );
  }
  if (typedefs.size === 0 || members.size === 0) {
    fail(
      `the carrier reported no '${CALLBACK_PREFIX}' rows ` +
        "(is the carrier test surface built?)",
    );
  }
  return { records, typedefs, members };
}

/** Splits a comma-separated signature list at the top level. */
function splitTopLevel(text) {
  const parts = [];
  let depth = 0;
  let current = "";
  for (const character of text) {
    if (character === "<") depth += 1;
    else if (character === ">") depth -= 1;
    if (character === "," && depth === 0) {
      parts.push(current);
      current = "";
      continue;
    }
    current += character;
  }
  parts.push(current);
  return parts.map((part) => part.trim()).filter((part) => part.length > 0);
}

const TYPE_TOKENS = /[A-Za-z_][A-Za-z0-9_]*|\*|[(),[\]]/g;

/** A C type spelled canonically: its tokens joined by single spaces. */
function canonicalType(text) {
  return (text.match(TYPE_TOKENS) ?? []).join(" ");
}

/**
 * A parameter type spelled canonically. A parameter name is not part of the
 * type, so a trailing identifier (when the parameter is more than just its
 * type) is dropped — `const uint8_t *data` and `const uint8_t *` agree.
 */
function canonicalParameter(text) {
  const tokens = text.match(TYPE_TOKENS) ?? [];
  if (tokens.length > 1 && /^[A-Za-z_][A-Za-z0-9_]*$/.test(tokens[tokens.length - 1])) {
    tokens.pop();
  }
  return tokens.join(" ");
}

/** The one canonical spelling both sides of the comparison are reduced to. */
function canonicalSignature(returnType, params) {
  return `(${params.map(canonicalParameter).join(", ")}) -> ${canonicalType(returnType)}`;
}

/** Renders one Rust type as the C type it must cross as. */
function renderRustType(rustType) {
  const text = rustType.trim();
  if (text === "()") return "void";
  const pointer = /^\*(mut|const) ([\s\S]+)$/.exec(text);
  if (pointer) {
    const inner = renderRustType(pointer[2]);
    return pointer[1] === "const" ? `const ${inner} *` : `${inner} *`;
  }
  const known = RUST_TYPES.get(text);
  if (known) return known;
  // A named type crosses by name; the module path it lives in is Rust-side.
  if (/^[A-Za-z_][A-Za-z0-9_:]*$/.test(text)) return text.split("::").pop();
  fail(`unparsed Rust type '${rustType}' in a carrier callback signature`);
}

/**
 * Renders one carrier-reported callback type as its C signature. The carrier
 * reports the type it actually has, so this compares that type against the
 * header rather than a second, hand-written copy of it.
 */
function renderCarrierSignature(rustType) {
  let text = rustType.trim();
  const option = /^core::option::Option<([\s\S]*)>$/.exec(text);
  if (option) text = option[1].trim();
  const callback = /^unsafe extern "C" fn\(([\s\S]*)\)(?: -> ([\s\S]+))?$/.exec(text);
  if (!callback) fail(`unparsed carrier callback type: '${rustType}'`);
  const params = splitTopLevel(callback[1]).map(renderRustType);
  const returnType = callback[2] ? renderRustType(callback[2]) : "void";
  return canonicalSignature(returnType, params);
}

/**
 * Parses the header's callback typedefs into `name → canonical C signature`.
 * Only `typedef struct` declarations are left to `parseRecords`: every other
 * `typedef` must be a callback typedef, so a declaration this gate cannot read
 * fails instead of going unverified.
 */
function parseCallbackTypedefs(headerText, headerPath) {
  const typedefs = new Map();
  for (const chunk of stripComments(headerText).split(";")) {
    const candidate = chunk.replace(/\s+/g, " ").trim();
    if (!candidate.startsWith("typedef ")) continue;
    if (/^typedef struct\b/.test(candidate)) continue;
    const match = CALLBACK_TYPEDEF_PATTERN.exec(candidate);
    if (!match) {
      fail(
        `${display(headerPath)}: unparsed typedef '${candidate.slice(0, 160)}' ` +
          "(a callback typedef is 'typedef <ret> (SPOKE_CONNECT_CALL *<Name>)(<params>)')",
      );
    }
    const [, returnType, name, params] = match;
    if (typedefs.has(name)) fail(`${display(headerPath)}: duplicate typedef '${name}'`);
    typedefs.set(name, canonicalSignature(returnType, splitTopLevel(params)));
  }
  if (typedefs.size === 0) fail(`${display(headerPath)}: no callback typedefs found`);
  return typedefs;
}

/** The callback signature a header record member declares, or `null`. */
function memberCallbackSignature(member, typedefs) {
  const inline = INLINE_CALLBACK_PATTERN.exec(member.text);
  if (inline) return canonicalSignature(inline[1], splitTopLevel(inline[3]));
  if (typedefs.has(member.type)) return typedefs.get(member.type);
  return null;
}

/**
 * Compares the header's callback typedefs and callback-table members against
 * the carrier's report and returns the mismatches. Size and offset comparisons
 * cannot see a callback whose parameters, return type or calling convention
 * changed — the pointer keeps its size and the table keeps its offsets — so
 * the signature itself is what is compared here, in both coverage directions.
 */
function compareCallbacks(headerText, headerPath, records, carrier) {
  const typedefs = parseCallbackTypedefs(headerText, headerPath);
  const mismatches = [];

  for (const [name, signature] of typedefs) {
    if (!carrier.typedefs.has(name)) {
      mismatches.push(`typedef ${name}: declared in the header, not reported by the carrier`);
      continue;
    }
    const reported = renderCarrierSignature(carrier.typedefs.get(name));
    if (reported !== signature) {
      mismatches.push(`typedef ${name}: header ${signature}, carrier ${reported}`);
    }
  }
  for (const name of carrier.typedefs.keys()) {
    if (!typedefs.has(name)) {
      mismatches.push(`typedef ${name}: reported by the carrier, not declared in the header`);
    }
  }

  const declared = new Map();
  for (const record of records) {
    for (const member of record.fields) {
      const signature = memberCallbackSignature(member, typedefs);
      if (signature) declared.set(`${record.name}.${member.name}`, signature);
    }
  }
  const reported = new Map();
  for (const [label, rustType] of carrier.members) {
    reported.set(label, renderCarrierSignature(rustType));
  }
  for (const [label, signature] of declared) {
    if (!reported.has(label)) {
      mismatches.push(`member ${label}: declared in the header, not reported by the carrier`);
      continue;
    }
    if (reported.get(label) !== signature) {
      mismatches.push(`member ${label}: header ${signature}, carrier ${reported.get(label)}`);
    }
  }
  for (const label of reported.keys()) {
    if (!declared.has(label)) {
      mismatches.push(`member ${label}: reported by the carrier, not declared in the header`);
    }
  }

  return { mismatches, typedefs, members: declared };
}

/** The callback comparison, reported the way the other checks report. */
function runCallbackCheck(headerText, headerPath, records, carrier) {
  const { mismatches, typedefs, members } = compareCallbacks(
    headerText,
    headerPath,
    records,
    carrier,
  );
  if (mismatches.length > 0) {
    console.error("Callback signatures: FAIL");
    for (const entry of mismatches) console.error(`  ${entry}`);
    process.exit(1);
  }
  console.log(
    `Callback signatures: PASS (${typedefs.size} typedefs, ${members.size} members ` +
      "match the carrier)",
  );
}

/** Compiles one probe translation unit; a non-zero exit is a gate failure. */
function runCompiler(entry, tempDir) {
  const result = spawnSync(entry.command, entry.args, { cwd: tempDir, encoding: "utf8" });
  if (result.error) {
    fail(`${entry.label}: failed to run '${entry.command}': ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(
      `${entry.label}: '${entry.command}' exited ${result.status}\n` +
        `${(result.stdout ?? "").trim()}\n${(result.stderr ?? "").trim()}`,
    );
  }
  console.log(`${entry.label}: PASS`);
}

/**
 * Pins every record the header declares to the layout the carrier's
 * `#[repr(C)]` mirror reports: one `_Static_assert` per `sizeof`, `_Alignof`
 * and `offsetof`, compiled against the header. Both coverage directions fail
 * the gate, so neither side can add, drop or move a record on its own.
 */
function runLayoutCheck(records, layouts, headerPath, tempDir) {
  const includeDir = dirname(headerPath);
  const lines = [
    "/* Generated by tooling/connect/cpp-symbol-check.mjs — do not edit. */",
    "/* Each assertion pins one header layout fact to the carrier's mirror. */",
    "#include <stddef.h>",
    '#include "spoke_connect.h"',
    "",
  ];
  const unreported = [];
  for (const record of records) {
    const layout = layouts.get(record.name);
    if (!layout) {
      unreported.push(record.name);
      continue;
    }
    lines.push(
      `_Static_assert(sizeof(${record.name}) == ${layout.size}, ` +
        `"${record.name}: size");`,
      `_Static_assert(_Alignof(${record.name}) == ${layout.align}, ` +
        `"${record.name}: alignment");`,
    );
    for (const field of record.fields) {
      const offset = layout.fields.get(field.name);
      if (offset === undefined) {
        unreported.push(`${record.name}.${field.name}`);
        continue;
      }
      lines.push(
        `_Static_assert(offsetof(${record.name}, ${field.name}) == ${offset}, ` +
          `"${record.name}.${field.name}: offset");`,
      );
    }
  }
  if (unreported.length > 0) {
    fail(
      `${display(headerPath)}: the carrier reports no layout for ` +
        `${unreported.length} declared entr${unreported.length === 1 ? "y" : "ies"}: ` +
        unreported.join(", "),
    );
  }

  const undeclared = [];
  for (const [name, layout] of layouts) {
    const record = records.find((candidate) => candidate.name === name);
    if (!record) {
      undeclared.push(name);
      continue;
    }
    for (const field of layout.fields.keys()) {
      if (!record.fields.some((candidate) => candidate.name === field)) {
        undeclared.push(`${name}.${field}`);
      }
    }
  }
  if (undeclared.length > 0) {
    fail(
      `the carrier reports layout for records/members ${display(headerPath)} ` +
        `does not declare: ${undeclared.join(", ")}`,
    );
  }

  const probeSource = join(tempDir, "spoke_connect_layout_probe.c");
  writeFileSync(probeSource, `${lines.join("\n")}\n`);
  const entry =
    process.platform === "win32"
      ? {
          label: "Record layout (cl.exe /std:c11 /W4 /WX)",
          command: "cl.exe",
          args: [
            "/nologo",
            "/TC",
            "/std:c11",
            "/W4",
            "/WX",
            `/I${includeDir}`,
            "/c",
            probeSource,
            `/Fo:${join(tempDir, "spoke_connect_layout_probe.obj")}`,
          ],
        }
      : {
          label: "Record layout (clang -std=c11 -Wall -Wextra -Werror)",
          command: "clang",
          args: [
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            `-I${includeDir}`,
            "-c",
            probeSource,
            "-o",
            join(tempDir, "spoke_connect_layout_probe.o"),
          ],
        };
  runCompiler(entry, tempDir);
  const asserted = lines.filter((line) => line.startsWith("_Static_assert")).length;
  console.log(
    `Record layout: ${records.length} records, ${asserted} assertions ` +
      `(sizeof/_Alignof/offsetof) match the carrier mirrors`,
  );
}

/** Enumerates the library's defined exported symbols in the reserved namespace. */
function exportedSymbols(libraryPath) {
  if (process.platform === "darwin") {
    const result = spawnSync("nm", ["-gU", libraryPath], { encoding: "utf8" });
    if (result.error) {
      fail(`failed to run 'nm': ${result.error.message}`);
    }
    if (result.status !== 0) {
      fail(`'nm -gU ${display(libraryPath)}' exited ${result.status}: ${result.stderr.trim()}`);
    }
    const symbols = new Set();
    for (const line of result.stdout.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      const fields = trimmed.split(/\s+/);
      if (fields.length < 3) {
        fail(`unparsed 'nm -gU' row: '${trimmed}'`);
      }
      const name = fields[fields.length - 1];
      // Mach-O prepends exactly one underscore to C symbols.
      const symbol = name.startsWith("_") ? name.slice(1) : name;
      if (symbol.startsWith(SYMBOL_NAMESPACE)) symbols.add(symbol);
    }
    return symbols;
  }

  if (process.platform === "win32") {
    const result = spawnSync("dumpbin", ["/nologo", "/exports", libraryPath], {
      encoding: "utf8",
    });
    if (result.error) {
      fail(
        `failed to run 'dumpbin' (is the MSVC developer environment on PATH?): ` +
          result.error.message,
      );
    }
    if (result.status !== 0) {
      fail(
        `'dumpbin /nologo /exports ${display(libraryPath)}' exited ${result.status}: ` +
          `${result.stderr.trim()}`,
      );
    }
    const lines = result.stdout.split("\n");
    const headerRow = lines.findIndex((line) =>
      /^\s*ordinal\s+hint\s+RVA\s+name\s*$/i.test(line),
    );
    if (headerRow < 0) {
      fail(`'dumpbin /exports' output has no export table header for ${display(libraryPath)}`);
    }
    const symbols = new Set();
    let sawExportRow = false;
    for (const line of lines.slice(headerRow + 1)) {
      const trimmed = line.trim();
      if (!trimmed) {
        // dumpbin may leave a blank separator between the table header and
        // its first row. Once rows begin, the blank line terminates the table
        // before the Summary section.
        if (sawExportRow) break;
        continue;
      }
      const match = /^\s*\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\S+)/.exec(line);
      if (!match) {
        fail(`unparsed 'dumpbin /exports' row: '${trimmed}'`);
      }
      sawExportRow = true;
      if (match[1].startsWith(SYMBOL_NAMESPACE)) symbols.add(match[1]);
    }
    if (!sawExportRow) {
      fail(`'dumpbin /exports' output has no export rows for ${display(libraryPath)}`);
    }
    return symbols;
  }

  fail(`unsupported platform '${process.platform}' for symbol enumeration`);
}

/** Two-way namespace diff. */
function diff(declarations, exports) {
  const declared = declarations.map((declaration) => declaration.name);
  const exported = [...exports];
  const missing = exported.filter((name) => !declared.includes(name)).sort();
  const extra = declared.filter((name) => !exports.has(name)).sort();
  return { declared, exported, missing, extra };
}

function reportFailure(result) {
  console.error("C ABI symbols: FAIL");
  console.error(
    `  declarations: ${result.declared.length}, exports: ${result.exported.length}`,
  );
  console.error(
    `  exported but not declared (${result.missing.length}): ` +
      `${result.missing.length > 0 ? result.missing.join(", ") : "(none)"}`,
  );
  console.error(
    `  declared but not exported (${result.extra.length}): ` +
      `${result.extra.length > 0 ? result.extra.join(", ") : "(none)"}`,
  );
}

/** Writes and compiles the declaration probes; returns nothing on success. */
function runProbes(declarations, headerPath, libraryPath, tempDir) {
  const includeDir = dirname(headerPath);
  const probeLibrary =
    process.platform === "win32" ? libraryPath.replace(/\.dll$/i, ".dll.lib") : libraryPath;
  if (process.platform === "win32" && !existsSync(probeLibrary)) {
    fail(`missing import library: ${display(probeLibrary)}`);
  }
  const lines = ["#include <stddef.h>", "#include <stdint.h>", '#include "spoke_connect.h"', ""];
  declarations.forEach((declaration, index) => {
    const params = declaration.params.replace(/\s+/g, " ").trim();
    lines.push(
      `typedef ${declaration.returnType} (SPOKE_CONNECT_CALL *spoke_connect_probe_t${index})(${params});`,
    );
    lines.push(
      `static volatile spoke_connect_probe_t${index} spoke_connect_probe_p${index} = ` +
        `&${declaration.name};`,
    );
  });
  lines.push("");
  lines.push("static void spoke_connect_probe_use_all(void) {");
  declarations.forEach((_, index) => {
    lines.push(`    (void)spoke_connect_probe_p${index};`);
  });
  lines.push("}");
  lines.push("");
  lines.push("int main(void) {");
  lines.push("    spoke_connect_probe_use_all();");
  lines.push("    return 0;");
  lines.push("}");
  lines.push("");
  const probeSource = join(tempDir, "spoke_connect_probe.c");
  writeFileSync(probeSource, lines.join("\n"));

  const cxxSource = join(tempDir, "spoke_connect_probe.cpp");
  writeFileSync(
    cxxSource,
    [
      '#include "spoke_connect.h"',
      '#include "spoke_connect.hpp"',
      "// Included twice: the convenience header must be safe to include more than once.",
      '#include "spoke_connect.hpp"',
      "",
      "#include <string_view>",
      "#include <utility>",
      "",
      "namespace {",
      "using AbiVersionFn = int32_t (SPOKE_CONNECT_CALL *)(uint64_t *, SpokeConnectError *);",
      "volatile AbiVersionFn abi_version_probe = &spoke_connect_abi_version;",
      "}  // namespace",
      "",
      "void spoke_connect_cxx_probe_use(void) { (void)abi_version_probe; }",
      "",
      "// Instantiates the convenience value layer and one move-only handle, so a",
      "// header that stopped compiling — or lost a name this layer wraps — fails",
      "// the gate instead of degrading to the C-only check.",
      "void spoke_connect_cxx_probe_convenience(void) {",
      "    namespace connect = spoke::connect;",
      "    connect::Result<connect::Buffer> buffered =",
      "        connect::Result<connect::Buffer>::success(connect::Buffer());",
      "    connect::Buffer taken = std::move(buffered).value();",
      "    const std::string_view view = taken.view();",
      "",
      "    connect::Result<connect::NonceStore> store = connect::NonceStore::create();",
      "    connect::NonceStore handle = std::move(store).value();",
      "    SpokeConnectNonceStore* raw = handle.release();",
      "    handle.adopt(raw);",
      "",
      "    connect::Result<void> bare = connect::Result<void>::success();",
      "    const SpokeConnectSlice borrowed = connect::slice(view);",
      "    const SpokeConnectSlice raw_bytes = connect::bytes(nullptr, 0);",
      "    (void)handle.get();",
      "    (void)bare.has_value();",
      "    (void)borrowed;",
      "    (void)raw_bytes;",
      "}",
      "",
    ].join("\n"),
  );

  const commands = [];
  if (process.platform === "win32") {
    commands.push({
      label: "C probe (c11, MSVC /MD)",
      command: "cl.exe",
      args: [
        "/nologo",
        "/TC",
        "/std:c11",
        "/MD",
        "/W4",
        "/WX",
        "/wd4232",
        `/I${includeDir}`,
        probeSource,
        `/Fe:${join(tempDir, "spoke_connect_probe.exe")}`,
        "/link",
        probeLibrary,
      ],
    });
    commands.push({
      label: "C++17 inclusion (no exceptions, no RTTI; .h + .hpp)",
      command: "cl.exe",
      args: [
        "/nologo",
        "/TP",
        "/std:c++17",
        "/EHs-c-",
        "/GR-",
        "/W4",
        "/WX",
        `/I${includeDir}`,
        "/c",
        cxxSource,
        `/Fo:${join(tempDir, "spoke_connect_probe_cxx.obj")}`,
      ],
    });
  } else {
    commands.push({
      label: "C probe (clang -std=c99 -Wall -Wextra -Werror)",
      command: "clang",
      args: [
        "-std=c99",
        "-Wall",
        "-Wextra",
        "-Werror",
        `-I${includeDir}`,
        probeSource,
        libraryPath,
        "-o",
        join(tempDir, "spoke_connect_probe"),
      ],
    });
    commands.push({
      label: "C++17 inclusion (no exceptions, no RTTI; .h + .hpp)",
      command: "clang++",
      args: [
        "-std=c++17",
        "-fno-exceptions",
        "-fno-rtti",
        "-Wall",
        "-Wextra",
        "-Werror",
        `-I${includeDir}`,
        "-c",
        cxxSource,
        "-o",
        join(tempDir, "spoke_connect_probe_cxx.o"),
      ],
    });
  }

  for (const entry of commands) {
    runCompiler(entry, tempDir);
  }
}

/** Symbol comparison only — used by the primary pass and the negative mutations. */
function compare(headerPath, libraryPath) {
  if (!existsSync(headerPath)) fail(`missing header: ${display(headerPath)}`);
  if (!existsSync(libraryPath)) fail(`missing library: ${display(libraryPath)}`);
  const declarations = parseDeclarations(readFileSync(headerPath, "utf8"), headerPath);
  const exports = exportedSymbols(libraryPath);
  if (exports.size === 0) {
    fail(
      `${display(libraryPath)} exports no '${SYMBOL_NAMESPACE}' symbols ` +
        "(empty export set)",
    );
  }
  return { declarations, result: diff(declarations, exports) };
}

function selfTest(headerPath, libraryPath, tempDir, carrier) {
  const headerText = readFileSync(headerPath, "utf8");
  const mutations = [
    {
      label: "missing declaration",
      expected: "missing",
      expectedSymbol: `${SYMBOL_NAMESPACE}abi_version`,
      text: (() => {
        const begin = headerText.indexOf(DECLARATIONS_BEGIN) + DECLARATIONS_BEGIN.length;
        const end = headerText.indexOf(DECLARATIONS_END);
        const body = headerText.slice(begin, end);
        const prototype = /SPOKE_CONNECT_API[^;]*spoke_connect_abi_version[^;]*;/;
        if (!prototype.test(body)) {
          fail("self-test: could not locate spoke_connect_abi_version to remove");
        }
        return (
          headerText.slice(0, begin) +
          body.replace(prototype, "") +
          headerText.slice(end)
        );
      })(),
    },
    {
      label: "invented declaration",
      expected: "extra",
      expectedSymbol: INVENTED_SYMBOL,
      text: headerText.replace(
        DECLARATIONS_END,
        `SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL ${INVENTED_SYMBOL}(void);\n${DECLARATIONS_END}`,
      ),
    },
  ];

  for (const mutation of mutations) {
    const mutated = join(tempDir, `spoke_connect_${mutation.expected}_header.h`);
    writeFileSync(mutated, mutation.text);
    const declarations = parseDeclarations(readFileSync(mutated, "utf8"), mutated);
    const exports = exportedSymbols(libraryPath);
    const result = diff(declarations, exports);
    const observed = mutation.expected === "missing" ? result.missing : result.extra;
    if (!observed.includes(mutation.expectedSymbol)) {
      reportFailure(result);
      fail(
        `self-test: the '${mutation.label}' mutation did not fail as expected ` +
          `(${mutation.expected} list: ${observed.length > 0 ? observed.join(", ") : "(empty)"})`,
      );
    }
    console.log(
      `negative mutation (${mutation.label}): FAILED as expected ` +
        `(${mutation.expected}: ${mutation.expectedSymbol})`,
    );
  }

  // A callback signature mutation: the borrowed `SpokeConnectSlice` argument
  // becomes an owned `SpokeConnectBuffer` of the same size, which keeps every
  // pointer size, every member offset and every declared symbol, so only the
  // signature comparison can fail on it.
  const mutatedText = headerText.replaceAll(
    "SpokeConnectSlice input_json,",
    "SpokeConnectBuffer input_json,",
  );
  const mutated = join(tempDir, "spoke_connect_callback_signature_header.h");
  writeFileSync(mutated, mutatedText);
  const { mismatches } = compareCallbacks(
    mutatedText,
    mutated,
    parseRecords(mutatedText, mutated),
    carrier,
  );
  if (mismatches.length === 0) {
    fail("self-test: the callback signature mutation did not fail as expected");
  }
  console.log(
    `negative mutation (callback signature): FAILED as expected (${mismatches[0]})`,
  );
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const { declarations, result } = compare(args.header, args.library);
  if (result.missing.length > 0 || result.extra.length > 0) {
    reportFailure(result);
    process.exit(1);
  }
  console.log(
    `C ABI symbols: PASS (${result.declared.length} declarations, ` +
      `${result.exported.length} exports, 0 missing, 0 extra)`,
  );

  const tempDir = mkdtempSync(join(tmpdir(), "spoke-connect-symbol-check-"));
  try {
    runProbes(declarations, args.header, args.library, tempDir);
    const headerText = readFileSync(args.header, "utf8");
    const records = parseRecords(headerText, args.header);
    const carrier = carrierReport();
    runLayoutCheck(records, carrier.records, args.header, tempDir);
    runCallbackCheck(headerText, args.header, records, carrier);
    if (args.selfTest) selfTest(args.header, args.library, tempDir, carrier);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

main();
