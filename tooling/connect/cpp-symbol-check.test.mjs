import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import { carrierReportJson, readCarrierReport, sortedObject } from "./cpp-symbol-check.mjs";

/**
 * Carrier-report serialization coverage. The compile side of the gate is
 * exercised in CI; here the file hand-off is held to byte parity, including
 * keys spelled '__proto__', which a plain-object accumulator silently drops.
 */
describe("sortedObject", () => {
  it("keeps '__proto__' as an own data property", () => {
    const serialized = sortedObject(
      new Map([
        ["zeta", "last"],
        ["__proto__", "first"],
      ]),
    );

    assert.ok(Object.prototype.hasOwnProperty.call(serialized, "__proto__"));
    assert.equal(serialized["__proto__"], "first");
    const parsed = JSON.parse(JSON.stringify(serialized));
    assert.ok(Object.prototype.hasOwnProperty.call(parsed, "__proto__"));
    assert.equal(parsed["__proto__"], "first");
    assert.deepEqual(Object.keys(parsed).sort(), ["__proto__", "zeta"]);
  });
});

describe("carrier report round-trip", () => {
  it("emits and re-reads a report byte-identically, __proto__ labels included", () => {
    const report = {
      records: new Map([
        [
          "Slice",
          {
            size: 16,
            align: 8,
            fields: new Map([
              ["__proto__", 0],
              ["len", 8],
            ]),
          },
        ],
        [
          "__proto__",
          {
            size: 1,
            align: 1,
            fields: new Map([["tag", 0]]),
          },
        ],
      ]),
      typedefs: new Map([
        ["__proto__", "void (*)(void *)"],
        ["on_data", "void (*)(void *, int32_t)"],
      ]),
      members: new Map([
        ["__proto__", "callback"],
        ["handle", "data"],
      ]),
      fieldTypes: new Map([
        ["__proto__", "uint8_t"],
        ["slice_len", "size_t"],
      ]),
    };

    const emitted = carrierReportJson(report);
    const dir = mkdtempSync(join(tmpdir(), "symbol-check-report-"));
    try {
      const path = join(dir, "carrier-report.json");
      writeFileSync(path, emitted);
      const reread = readCarrierReport(path);

      // Byte parity: emit(read(emit(x))) === emit(x).
      assert.equal(carrierReportJson(reread), emitted);
      // And the dangerous labels survive as keys, not prototype writes.
      assert.equal(reread.typedefs.get("__proto__"), "void (*)(void *)");
      assert.equal(reread.members.get("__proto__"), "callback");
      assert.equal(reread.fieldTypes.get("__proto__"), "uint8_t");
      assert.equal(reread.records.get("Slice").fields.get("__proto__"), 0);
      assert.equal(reread.records.get("__proto__").size, 1);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
