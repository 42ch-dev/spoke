// Localize + dereference a schema tree (mirrors run.mjs steps) for typify experiments.
import $RefParser from "@apidevtools/json-schema-ref-parser";
import { glob } from "glob";
import fs from "node:fs";
import path from "node:path";

const [SRC, OUT] = process.argv.slice(2).map((p) => path.resolve(p));
const BASE = "https://spoke42.invalid/schemas/";

function localize(obj, relPath) {
  obj.$id = relPath;
  const dir = path.posix.dirname(relPath);
  const visit = (node) => {
    if (Array.isArray(node)) return node.forEach(visit);
    if (node && typeof node === "object") {
      if (typeof node.$ref === "string" && node.$ref.startsWith(BASE)) {
        const [filePart, fragment] = node.$ref.slice(BASE.length).split("#");
        let localized = path.posix.relative(dir, filePart);
        if (!localized.startsWith(".")) localized = `./${localized}`;
        node.$ref = fragment ? `${localized}#${fragment}` : localized;
      }
      Object.values(node).forEach(visit);
    }
  };
  visit(obj);
  return obj;
}

fs.rmSync(OUT, { recursive: true, force: true });
const files = (await glob("**/*.schema.json", { cwd: SRC })).sort();
for (const rel of files) {
  const raw = JSON.parse(fs.readFileSync(path.join(SRC, rel), "utf8"));
  raw.$id = path.join(SRC, rel);
  const bundled = await $RefParser.dereference(localize(raw, rel));
  const out = path.join(OUT, rel);
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, JSON.stringify(bundled, null, 2));
}
console.log("dereferenced", files.length, "files");
