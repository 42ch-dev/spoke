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
 *
 * The compared namespace is `spoke_connect_` only: the carrier links the
 * `spoke-connect` crate, whose UniFFI scaffolding (`uniffi_spoke_connect_*`,
 * `ffi_spoke_connect_*`) is a different ABI and not a consumer entry point.
 *
 * Usage:
 *   node tooling/connect/cpp-symbol-check.mjs --header <header> --library <native>
 *   node tooling/connect/cpp-symbol-check.mjs --header <header> --library <native> --self-test
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
        probeLibrary,
        `/Fe:${join(tempDir, "spoke_connect_probe.exe")}`,
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
    if (args.selfTest) selfTest(args.header, args.library, tempDir);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

main();
