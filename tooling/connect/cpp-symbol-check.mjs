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
 *      /WX`), and the header must also compile in C++17 mode with exceptions
 *      and RTTI disabled.
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
 * copies with (a) a real declaration removed and (b) an invented declaration
 * added must both fail the symbol comparison. Temporary files are removed on
 * success and on failure.
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
const LAYOUT_TEST = "abi_layout";
const RECORD_PATTERN =
  /typedef\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{([^{}]*)\}\s*([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
/** The other legal record declaration: an opaque handle with no layout. */
const OPAQUE_PATTERN =
  /typedef\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
const STRUCT_START_PATTERN = /typedef\s+struct\b/g;

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
      fields.push(name);
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
 * Runs the carrier's layout report (`crates/spoke-connect-capi/src/
 * abi_layout.rs`) and parses it into `name → {size, align, fields}`. An empty
 * report fails: without it the layout comparison would pass vacuously.
 */
function layoutReport() {
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
  for (const line of `${result.stdout ?? ""}\n${result.stderr ?? ""}`.split("\n")) {
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
  return records;
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
      const offset = layout.fields.get(field);
      if (offset === undefined) {
        unreported.push(`${record.name}.${field}`);
        continue;
      }
      lines.push(
        `_Static_assert(offsetof(${record.name}, ${field}) == ${offset}, ` +
          `"${record.name}.${field}: offset");`,
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
      if (!record.fields.includes(field)) undeclared.push(`${name}.${field}`);
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
      "",
      "namespace {",
      "using AbiVersionFn = int32_t (SPOKE_CONNECT_CALL *)(uint64_t *, SpokeConnectError *);",
      "volatile AbiVersionFn abi_version_probe = &spoke_connect_abi_version;",
      "}  // namespace",
      "",
      "void spoke_connect_cxx_probe_use(void) { (void)abi_version_probe; }",
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
      label: "C++17 inclusion (no exceptions, no RTTI)",
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
      label: "C++17 inclusion (no exceptions, no RTTI)",
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

function selfTest(headerPath, libraryPath, tempDir) {
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
    runLayoutCheck(
      parseRecords(readFileSync(args.header, "utf8"), args.header),
      layoutReport(),
      args.header,
      tempDir,
    );
    if (args.selfTest) selfTest(args.header, args.library, tempDir);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

main();
