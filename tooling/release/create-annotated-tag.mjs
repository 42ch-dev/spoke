#!/usr/bin/env node
/**
 * Create an annotated tag object + refs/tags/vX.Y.Z via the Git Data API.
 *
 * Used by New release after a GitHub-signed commit lands on main. Prefer this
 * over `git tag` + `git push` so the workflow stays token-API based.
 *
 * CLI:
 *   GITHUB_TOKEN=… node tooling/release/create-annotated-tag.mjs \
 *     --tag v0.1.0 --commit <sha> [--message "Release v0.1.0"] [--repo owner/name]
 *
 * @module tooling/release/create-annotated-tag
 */

/**
 * @param {string[]} argv
 */
function parseArgs(argv) {
  /** @type {{ tag: string | null; commit: string | null; message: string | null; repo: string | null }} */
  const out = {
    tag: null,
    commit: null,
    message: null,
    repo: process.env.GITHUB_REPOSITORY ?? null,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--tag") {
      out.tag = argv[++i] ?? null;
    } else if (arg === "--commit") {
      out.commit = argv[++i] ?? null;
    } else if (arg === "--message") {
      out.message = argv[++i] ?? null;
    } else if (arg === "--repo") {
      out.repo = argv[++i] ?? null;
    } else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: create-annotated-tag.mjs --tag vX.Y.Z --commit <sha> [--message <text>] [--repo owner/name]",
      );
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (!out.tag || !out.commit || !out.repo?.includes("/")) {
    console.error("Required: --tag, --commit, and --repo or GITHUB_REPOSITORY");
    process.exit(1);
  }

  if (!out.tag.startsWith("v")) {
    console.error(`Tag must start with v (got ${out.tag})`);
    process.exit(1);
  }

  out.message = out.message ?? `Release ${out.tag}`;
  return /** @type {{ tag: string; commit: string; message: string; repo: string }} */ (
    out
  );
}

/**
 * @param {string} path
 * @param {Record<string, unknown>} [body]
 * @param {string} [method]
 */
async function ghApi(path, body, method = body === undefined ? "GET" : "POST") {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (!token) {
    throw new Error("GITHUB_TOKEN (or GH_TOKEN) is required");
  }

  const response = await fetch(`https://api.github.com${path}`, {
    method,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  const text = await response.text();
  /** @type {unknown} */
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = text;
    }
  }

  if (!response.ok) {
    throw new Error(
      `GitHub API ${method} ${path} → ${response.status}: ${typeof data === "string" ? data : JSON.stringify(data)}`,
    );
  }

  return data;
}

async function main() {
  const { tag, commit, message, repo } = parseArgs(process.argv.slice(2));

  // Idempotent: if annotated tag already points at this commit, succeed.
  try {
    const existing = await ghApi(`/repos/${repo}/git/ref/tags/${tag}`);
    if (
      existing &&
      typeof existing === "object" &&
      "object" in existing &&
      existing.object &&
      typeof existing.object === "object" &&
      "sha" in existing.object
    ) {
      const tip = String(existing.object.sha);
      if (
        "type" in existing.object &&
        existing.object.type === "commit" &&
        tip === commit
      ) {
        console.log(`Tag ${tag} already at ${commit}`);
        return;
      }
      if ("type" in existing.object && existing.object.type === "tag") {
        const tagObj = await ghApi(`/repos/${repo}/git/tags/${tip}`);
        if (
          tagObj &&
          typeof tagObj === "object" &&
          "object" in tagObj &&
          tagObj.object &&
          typeof tagObj.object === "object" &&
          "sha" in tagObj.object &&
          String(tagObj.object.sha) === commit
        ) {
          console.log(`Annotated tag ${tag} already at ${commit}`);
          return;
        }
      }
      throw new Error(
        `Tag ${tag} already exists and does not point at ${commit}`,
      );
    }
  } catch (error) {
    const messageText = error instanceof Error ? error.message : String(error);
    if (!/\b404\b/.test(messageText)) {
      throw error;
    }
  }

  const taggerDate = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  const created = await ghApi(`/repos/${repo}/git/tags`, {
    tag,
    message,
    object: commit,
    type: "commit",
    tagger: {
      name: "github-actions[bot]",
      email: "41898282+github-actions[bot]@users.noreply.github.com",
      date: taggerDate,
    },
  });

  if (!created || typeof created !== "object" || !("sha" in created)) {
    throw new Error(`Unexpected tag create response: ${JSON.stringify(created)}`);
  }

  await ghApi(`/repos/${repo}/git/refs`, {
    ref: `refs/tags/${tag}`,
    sha: created.sha,
  });

  console.log(
    `Created annotated tag ${tag} → ${commit} (tag object ${created.sha})`,
  );
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
