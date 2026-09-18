/**
 * Lockstep version surfaces — SSOT manifest for assert and bump scripts.
 *
 * Normative source: `.mstar/specs/spoke-version-release.md` lockstep table (rows 1–16).
 *
 * Excluded from lockstep (documented only; not asserted):
 * - tooling/codegen/rust-gen/Cargo.toml — standalone codegen bin crate; not a consumer pin surface.
 * - pnpm-lock.yaml — workspace `link:` entries do not embed package SemVer.
 */

/** @type {string} Canonical version source (row 1). */
export const CANONICAL_PATH = "package.json";

/**
 * package.json files whose top-level `version` must match canonical (rows 2–5
 * plus the workspace-private connect TS package).
 * @type {readonly string[]}
 */
export const JSON_VERSION_PATHS = [
  "packages/spoke-schemas/package.json",
  "packages/spoke-operations/package.json",
  "fixtures/toy-world/package.json",
  "tooling/codegen/package.json",
  "packages/spoke-connect-ts/package.json",
];

/** @type {string} Cargo workspace version (row 6). */
export const CARGO_WORKSPACE_PATH = "Cargo.toml";


/**
 * C# NuGet project Version (GitHub Packages 42ch.Spoke.Connect; lockstep).
 * @type {string}
 */
export const NUGET_CONNECT_CSPROJ_PATH =
  "crates/spoke-connect/bindings/csharp/42ch.Spoke.Connect.csproj";

/**
 * Python PyPI project version (PyPI `spoke-connect`; lockstep).
 * @type {string}
 */
export const PYPI_CONNECT_PYPROJECT_PATH =
  "crates/spoke-connect/bindings/python/pyproject.toml";

/**
 * Kotlin Maven project version (GitHub Packages dev.42ch:spoke-connect;
 * lockstep).
 * @type {string}
 */
export const MAVEN_CONNECT_GRADLE_PATH =
  "crates/spoke-connect/bindings/kotlin/build.gradle.kts";

/** @type {string} Cargo lockfile — workspace member package versions (row 10). */
export const CARGO_LOCK_PATH = "Cargo.lock";

/**
 * Read `<Version>X.Y.Z</Version>` from a SDK-style csproj.
 * @param {string} contents
 * @returns {string | null}
 */
export function parseCsprojVersion(contents) {
  const match = contents.match(/<Version>\s*([^<]+?)\s*<\/Version>/);
  return match?.[1]?.trim() ?? null;
}

/**
 * Replace the first `<Version>…</Version>` in a SDK-style csproj.
 * @param {string} contents
 * @param {string} version
 * @param {string} manifestPath
 * @returns {string}
 */
export function replaceCsprojVersion(contents, version, manifestPath) {
  if (!/<Version>\s*[^<]+?\s*<\/Version>/.test(contents)) {
    throw new Error(`${manifestPath}: missing <Version>…</Version>`);
  }
  return contents.replace(
    /<Version>\s*[^<]+?\s*<\/Version>/,
    `<Version>${version}</Version>`,
  );
}

/**
 * Read `version = "X.Y.Z"` from the `[project]` table in pyproject.toml.
 * @param {string} contents
 * @returns {string | null}
 */
export function parsePyprojectVersion(contents) {
  const projectSection = contents.match(/\[project\][\s\S]*?(?=\n\[|$)/);
  if (!projectSection) {
    return null;
  }
  const match = projectSection[0].match(/^version\s*=\s*"([^"]+)"/m);
  return match?.[1]?.trim() ?? null;
}

/**
 * Replace `version = "…"` inside the `[project]` table.
 * @param {string} contents
 * @param {string} version
 * @param {string} manifestPath
 * @returns {string}
 */
export function replacePyprojectVersion(contents, version, manifestPath) {
  const projectSection = contents.match(/\[project\][\s\S]*?(?=\n\[|$)/);
  if (!projectSection || !/^version\s*=\s*"[^"]+"/m.test(projectSection[0])) {
    throw new Error(`${manifestPath}: missing [project] version = "…"`);
  }
  return contents.replace(
    /(\[project\][\s\S]*?^version\s*=\s*")[^"]+(")/m,
    `$1${version}$2`,
  );
}

/**
 * Read `version = "X.Y.Z"` from a Kotlin Gradle build script.
 * @param {string} contents
 * @returns {string | null}
 */
export function parseGradleVersion(contents) {
  const match = contents.match(/^version\s*=\s*"([^"]+)"/m);
  return match?.[1]?.trim() ?? null;
}

/**
 * Replace the first `version = "…"` in a Gradle build script.
 * @param {string} contents
 * @param {string} version
 * @param {string} manifestPath
 * @returns {string}
 */
export function replaceGradleVersion(contents, version, manifestPath) {
  if (!/^version\s*=\s*"[^"]+"/m.test(contents)) {
    throw new Error(`${manifestPath}: missing version = "…"`);
  }
  return contents.replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`);
}

/**
 * Parse Cargo workspace member paths from the `[workspace]` `members` array.
 *
 * @param {string} contents
 * @returns {string[]}
 */
export function parseCargoWorkspaceMembers(contents) {
  const workspaceStart = contents.indexOf("[workspace]");
  const workspaceSection =
    workspaceStart < 0
      ? null
      : contents.slice(workspaceStart + "[workspace]".length);
  const membersBody = workspaceSection?.match(
    /^\s*members\s*=\s*\[([\s\S]*?)\]/m,
  )?.[1];
  if (membersBody === undefined) {
    throw new Error("Cargo.toml: missing [workspace].members array");
  }
  const memberPaths = [
    ...membersBody.replace(/#.*$/gm, "").matchAll(/"([^"]+)"/g),
  ].map(([, memberPath]) => memberPath);
  for (const memberPath of memberPaths) {
    if (/[*?\[\]]/.test(memberPath)) {
      throw new Error(
        `Cargo.toml: workspace member "${memberPath}" uses glob metacharacters; explicit member paths are required`,
      );
    }
  }
  return memberPaths;
}

/**
 * Parse the package name from a member's Cargo manifest.
 *
 * @param {string} contents
 * @returns {string}
 */
export function parseCargoPackageName(contents) {
  const packageStart = contents.indexOf("[package]");
  const packageSection =
    packageStart < 0
      ? null
      : contents.slice(packageStart + "[package]".length);
  const packageName = packageSection?.match(
    /^\s*name\s*=\s*"([^"]+)"/m,
  )?.[1];
  if (!packageName) {
    throw new Error("Cargo.toml: missing [package].name");
  }
  return packageName;
}

/**
 * Resolve Cargo lock package names from workspace members.
 *
 * @param {string} workspaceContents
 * @param {(memberPath: string) => string} readMemberManifest
 * @returns {string[]}
 */
export function resolveCargoLockPackageNames(
  workspaceContents,
  readMemberManifest,
) {
  return parseCargoWorkspaceMembers(workspaceContents).map((memberPath) =>
    parseCargoPackageName(readMemberManifest(memberPath)),
  );
}

const CARGO_STRING_PATTERN = String.raw`(?:"([^"\\]*(?:\\.[^"\\]*)*)"|'([^']*)')`;

/**
 * Refuse Cargo dependency declaration forms that this narrow scanner cannot
 * parse safely.
 *
 * @param {string} contents
 * @param {readonly string[] | ReadonlySet<string>} packageNames
 * @param {string} manifestPath
 */
function assertSupportedCargoPathDependencyShape(
  contents,
  packageNames,
  manifestPath,
) {
  const workspaceMembers = new Set(packageNames);
  const sectionPattern =
    /^\s*\[(dependencies|dev-dependencies|build-dependencies)\.([A-Za-z0-9_-]+)\]\s*(?:#.*)?$/gm;
  for (const [, family, member] of contents.matchAll(sectionPattern)) {
    if (workspaceMembers.has(member)) {
      throw new Error(
        `${manifestPath}: dependency section [${family}.${member}] for workspace member "${member}" is unsupported; inline dependency tables are the supported shape`,
      );
    }
  }

  const dottedKeyPattern =
    /^\s*([A-Za-z0-9_-]+)\s*\.\s*(version|path)\s*=/gm;
  for (const [, member, attribute] of contents.matchAll(dottedKeyPattern)) {
    if (workspaceMembers.has(member)) {
      throw new Error(
        `${manifestPath}: dotted-key dependency ${member}.${attribute} for workspace member "${member}" is unsupported; inline dependency tables are the supported shape`,
      );
    }
  }
}

/**
 * Read a quoted Cargo inline-table attribute.
 *
 * @param {string} attributes
 * @param {string} key
 * @returns {{ value: string; quoted: string } | null}
 */
function readCargoStringAttribute(attributes, key) {
  const match = attributes.match(
    new RegExp(`\\b${key}\\s*=\\s*(${CARGO_STRING_PATTERN})`),
  );
  if (!match) {
    return null;
  }
  const quoted = match[1];
  return {
    quoted,
    value: quoted.startsWith('"') ? match[2] : match[3],
  };
}

/**
 * Parse inline dependency tables that pin workspace crates by both version and
 * path. The returned name uses the inline `package` attribute for aliases,
 * falling back to the dependency key. Path-only dependencies intentionally do
 * not participate in lockstep.
 *
 * @param {string} contents
 * @param {readonly string[] | ReadonlySet<string>} packageNames
 * @param {string} manifestPath
 * @returns {{ name: string; version: string; path: string }[]}
 */
export function parseCargoPathDependencyPins(
  contents,
  packageNames,
  manifestPath,
) {
  assertSupportedCargoPathDependencyShape(contents, packageNames, manifestPath);
  const pins = [];
  const dependencyPattern =
    /^[ \t]*([A-Za-z0-9_-]+)\s*=\s*\{([^{}]*)\}/gm;
  for (const [, name, attributes] of contents.matchAll(dependencyPattern)) {
    const version = readCargoStringAttribute(attributes, "version");
    const path = readCargoStringAttribute(attributes, "path");
    const packageName = readCargoStringAttribute(attributes, "package");
    if (version !== null && path !== null) {
      pins.push({
        name: packageName?.value ?? name,
        version: version.value,
        path: path.value,
      });
    }
  }
  return pins;
}

/**
 * Rewrite every selected inline workspace path dependency's version.
 * Missing or path-only dependencies are left untouched.
 *
 * @param {string} contents
 * @param {string} version
 * @param {readonly string[]} packageNames
 * @param {string} manifestPath
 * @returns {string}
 */
export function replaceCargoPathDependencyPinVersions(
  contents,
  version,
  packageNames,
  manifestPath,
) {
  assertSupportedCargoPathDependencyShape(contents, packageNames, manifestPath);
  const selected = new Set(packageNames);
  return contents.replace(
    /^([ \t]*)([A-Za-z0-9_-]+)\s*=\s*\{([^{}]*)\}/gm,
    (full, indentation, name, attributes) => {
      const packageName = readCargoStringAttribute(attributes, "package");
      const path = readCargoStringAttribute(attributes, "path");
      const versionPattern = new RegExp(
        `(\\bversion\\s*=\\s*)(${CARGO_STRING_PATTERN})`,
      );
      const currentVersion = attributes.match(versionPattern);
      if (
        !selected.has(packageName?.value ?? name) ||
        path === null ||
        currentVersion === null
      ) {
        return full;
      }
      const quote = currentVersion[3] !== undefined ? '"' : "'";
      const updatedVersion = `${currentVersion[1]}${quote}${version}${quote}`;
      return full.replace(currentVersion[0], updatedVersion);
    },
  );
}


/**
 * Read the version for a top-level `[[package]]` entry in Cargo.lock.
 *
 * @param {string} contents
 * @param {string} packageName
 * @returns {string | null}
 */
export function parseCargoLockPackageVersion(contents, packageName) {
  const escaped = packageName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = contents.match(
    new RegExp(
      `\\[\\[package\\]\\]\\s*\\nname\\s*=\\s*"${escaped}"\\s*\\nversion\\s*=\\s*"([^"]+)"`,
    ),
  );
  return match?.[1] ?? null;
}

/**
 * Rewrite workspace member package versions in Cargo.lock to match lockstep.
 * Does not invoke `cargo` (New release CI is Node-only). Idempotent when the
 * lockfile already lists the target version.
 *
 * @param {string} contents
 * @param {string} version
 * @param {readonly string[]} packageNames Derived from Cargo.toml workspace members.
 * @returns {string}
 */
export function replaceCargoLockPackageVersions(
  contents,
  version,
  packageNames,
) {
  let updated = contents;
  for (const packageName of packageNames) {
    const current = parseCargoLockPackageVersion(updated, packageName);
    if (current === null) {
      throw new Error(
        `${CARGO_LOCK_PATH}: missing [[package]] entry for ${packageName}`,
      );
    }
    if (current === version) {
      continue;
    }
    const escaped = packageName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const pattern = new RegExp(
      `(\\[\\[package\\]\\]\\s*\\nname\\s*=\\s*"${escaped}"\\s*\\nversion\\s*=\\s*")[^"]+(")`,
    );
    const next = updated.replace(pattern, `$1${version}$2`);
    if (next === updated) {
      throw new Error(
        `${CARGO_LOCK_PATH}: could not update [[package]] version for ${packageName}`,
      );
    }
    updated = next;
  }
  return updated;
}

/**
 * README files that must carry the dynamic GitHub Releases version badge.
 * @type {readonly string[]}
 */
export const README_BADGE_PATHS = ["README.md", "README_CN.md"];

/**
 * Dynamic shields.io GitHub Releases badge (includes prereleases, SemVer sort).
 * Not rewritten on bump — tracks the latest GitHub Release for the repo.
 */
export const README_RELEASE_BADGE_MARKER =
  "https://img.shields.io/github/v/release/42ch-dev/spoke";

/**
 * True when README content embeds the dynamic GitHub Releases shields badge.
 * Parses URL tokens and matches host + path (not a raw substring check).
 *
 * @param {string} contents
 * @returns {boolean}
 */
export function hasReadmeReleaseBadge(contents) {
  const expected = new URL(README_RELEASE_BADGE_MARKER);
  const urlRe = /https?:\/\/[^\s)"'\]]+/g;
  for (const match of contents.matchAll(urlRe)) {
    try {
      const parsed = new URL(match[0]);
      if (
        parsed.hostname === expected.hostname &&
        parsed.pathname === expected.pathname
      ) {
        return true;
      }
    } catch {
      // Ignore malformed URL-like tokens in markdown.
    }
  }
  return false;
}
