import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  CARGO_LOCK_PATH,
  CARGO_WORKSPACE_PATH,
  parseCargoPathDependencyPins,
  parseCargoWorkspaceMembers,
  replaceCargoLockPackageVersions,
  replaceCargoPathDependencyPinVersions,
  resolveCargoLockPackageNames,
} from "../release/lockstep-surfaces.mjs";

const [, , command, repoRootArg, backupDirArg, version] = process.argv;
if (!command || !repoRootArg || !backupDirArg) {
  throw new Error(
    "Usage: normalize-cargo-version.mjs <apply|restore> <repo-root> <backup-dir> [version]",
  );
}

const repoRoot = resolve(repoRootArg);
const backupDir = resolve(backupDirArg);
const workspaceRelativePath = CARGO_WORKSPACE_PATH;
const lockRelativePath = CARGO_LOCK_PATH;

function readRepoFile(relativePath) {
  return readFileSync(join(repoRoot, relativePath), "utf8");
}

function replaceWorkspacePackageVersion(contents, targetVersion) {
  const sectionMatch = contents.match(
    /(\[workspace\.package\][\s\S]*?)(?=\n\[|\s*$)/,
  );
  if (!sectionMatch) {
    throw new Error(
      `${CARGO_WORKSPACE_PATH}: missing [workspace.package] section`,
    );
  }

  const updatedSection = sectionMatch[1].replace(
    /^version\s*=\s*"[^"]*"/m,
    `version = "${targetVersion}"`,
  );
  if (updatedSection === sectionMatch[1]) {
    throw new Error(
      `${CARGO_WORKSPACE_PATH}: could not find version = "..." in [workspace.package]`,
    );
  }
  return contents.replace(sectionMatch[1], updatedSection);
}

function apply() {
  if (!version) {
    throw new Error("apply requires a sentinel version");
  }

  const workspaceContents = readRepoFile(workspaceRelativePath);
  const memberPaths = parseCargoWorkspaceMembers(workspaceContents);
  const packageNames = resolveCargoLockPackageNames(
    workspaceContents,
    (memberPath) => readRepoFile(join(memberPath, "Cargo.toml")),
  );
  const relativePaths = [
    workspaceRelativePath,
    lockRelativePath,
    ...memberPaths.map((memberPath) => join(memberPath, "Cargo.toml")),
  ];
  const originals = new Map(
    relativePaths.map((relativePath) => [
      relativePath,
      readRepoFile(relativePath),
    ]),
  );

  const updated = new Map();
  updated.set(
    workspaceRelativePath,
    replaceWorkspacePackageVersion(workspaceContents, version),
  );
  updated.set(
    lockRelativePath,
    replaceCargoLockPackageVersions(
      originals.get(lockRelativePath),
      version,
      packageNames,
    ),
  );
  for (const memberPath of memberPaths) {
    const relativePath = join(memberPath, "Cargo.toml");
    const contents = originals.get(relativePath);
    parseCargoPathDependencyPins(contents, packageNames, relativePath);
    updated.set(
      relativePath,
      replaceCargoPathDependencyPinVersions(
        contents,
        version,
        packageNames,
        relativePath,
      ),
    );
  }

  mkdirSync(backupDir, { recursive: true });
  writeFileSync(
    join(backupDir, "files.json"),
    `${JSON.stringify(relativePaths)}\n`,
    "utf8",
  );
  for (const relativePath of relativePaths) {
    const sourcePath = join(repoRoot, relativePath);
    const backupPath = join(backupDir, relativePath);
    mkdirSync(resolve(backupPath, ".."), { recursive: true });
    copyFileSync(sourcePath, backupPath);
  }
  for (const [relativePath, contents] of updated) {
    writeFileSync(join(repoRoot, relativePath), contents, "utf8");
  }
}

function restore() {
  const relativePaths = JSON.parse(
    readFileSync(join(backupDir, "files.json"), "utf8"),
  );
  for (const relativePath of relativePaths) {
    try {
      copyFileSync(join(backupDir, relativePath), join(repoRoot, relativePath));
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw new Error(`restore failed for ${relativePath}: ${detail}`);
    }
  }
}

if (command === "apply") {
  apply();
} else if (command === "restore") {
  restore();
} else {
  throw new Error(`Unknown command: ${command}`);
}
