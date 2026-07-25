#!/usr/bin/env node
/**
 * Push working-tree changes as a GitHub-verified commit via GraphQL
 * `createCommitOnBranch` (no bot GPG key required).
 *
 * Satisfies repository `required_signatures` rulesets. Local `git commit` +
 * `git push` produces unsigned commits that cannot merge to protected main.
 *
 * CLI:
 *   GITHUB_TOKEN=… node tooling/release/push-github-signed-commit.mjs \
 *     --branch release/X.Y.Z \
 *     --message "chore(release): bump version to X.Y.Z" \
 *     [--base-ref main] \
 *     [--repo owner/name]
 *
 * Resets the remote branch tip to `--base-ref`, then commits the diff of the
 * current working tree / index against that base as one signed commit.
 *
 * @module tooling/release/push-github-signed-commit
 */

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");

/**
 * @param {string[]} args
 * @returns {string}
 */
function git(args) {
  return execFileSync("git", args, {
    cwd: REPO_ROOT,
    encoding: "utf8",
  }).trim();
}

/**
 * @param {string[]} argv
 */
function parseArgs(argv) {
  /** @type {{ branch: string | null; message: string | null; baseRef: string; repo: string | null }} */
  const out = {
    branch: null,
    message: null,
    baseRef: "main",
    repo: process.env.GITHUB_REPOSITORY ?? null,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--branch") {
      out.branch = argv[++i] ?? null;
    } else if (arg === "--message") {
      out.message = argv[++i] ?? null;
    } else if (arg === "--base-ref") {
      out.baseRef = argv[++i] ?? out.baseRef;
    } else if (arg === "--repo") {
      out.repo = argv[++i] ?? null;
    } else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: push-github-signed-commit.mjs --branch <name> --message <headline> [--base-ref main] [--repo owner/name]",
      );
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (!out.branch || !out.message) {
    console.error("Required: --branch and --message");
    process.exit(1);
  }
  if (!out.repo || !out.repo.includes("/")) {
    console.error(
      "Set --repo owner/name or GITHUB_REPOSITORY (e.g. 42ch-dev/spoke)",
    );
    process.exit(1);
  }

  return /** @type {{ branch: string; message: string; baseRef: string; repo: string }} */ (
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

/**
 * @param {string} query
 * @param {Record<string, unknown>} variables
 */
async function ghGraphql(query, variables) {
  const data = await ghApi("/graphql", { query, variables });
  if (
    data &&
    typeof data === "object" &&
    "errors" in data &&
    Array.isArray(data.errors) &&
    data.errors.length > 0
  ) {
    throw new Error(`GraphQL errors: ${JSON.stringify(data.errors)}`);
  }
  return data;
}

/**
 * Collect additions/deletions for createCommitOnBranch from working tree vs base.
 *
 * @param {string} baseRef
 * @returns {{ additions: { path: string; contents: string }[]; deletions: { path: string }[] }}
 */
function collectFileChanges(baseRef) {
  const raw = git(["diff", "--name-status", "--find-renames", baseRef]);
  /** @type {{ path: string; contents: string }[]} */
  const additions = [];
  /** @type {{ path: string }[]} */
  const deletions = [];

  if (!raw) {
    return { additions, deletions };
  }

  for (const line of raw.split("\n")) {
    if (!line) {
      continue;
    }
    const parts = line.split("\t");
    const status = parts[0] ?? "";
    const code = status[0];

    if (code === "D") {
      const path = parts[1];
      if (path) {
        deletions.push({ path });
      }
      continue;
    }

    if (code === "R" || code === "C") {
      const from = parts[1];
      const to = parts[2];
      if (code === "R" && from) {
        deletions.push({ path: from });
      }
      if (to && existsSync(join(REPO_ROOT, to))) {
        additions.push({
          path: to,
          contents: readFileSync(join(REPO_ROOT, to)).toString("base64"),
        });
      }
      continue;
    }

    // A / M / T / etc.
    const path = parts[1];
    if (!path) {
      continue;
    }
    if (!existsSync(join(REPO_ROOT, path))) {
      deletions.push({ path });
      continue;
    }
    additions.push({
      path,
      contents: readFileSync(join(REPO_ROOT, path)).toString("base64"),
    });
  }

  return { additions, deletions };
}

/**
 * Point remote branch at baseOid (create or force-update). Release branches are
 * outside ~DEFAULT_BRANCH, so force updates are allowed.
 *
 * IMPORTANT: `GET /git/refs/heads/<prefix>` is a **prefix** match (plural `refs`).
 * `release/0.1.0` matches `release/0.1.0-alpha.3` and returns HTTP 200 — do not use
 * that to decide existence. Use singular `GET /git/ref/heads/<branch>` (exact).
 *
 * @param {string} repo
 * @param {string} branch
 * @param {string} baseOid
 */
async function ensureBranchAtOid(repo, branch, baseOid) {
  // Singular "ref" = exact match. Plural "refs" = prefix match (unsafe here).
  const exactRefPath = `/repos/${repo}/git/ref/heads/${branch}`;
  const updateRefPath = `/repos/${repo}/git/refs/heads/${branch}`;
  let exists = false;
  try {
    await ghApi(exactRefPath);
    exists = true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (!/\b404\b/.test(message) && !/Reference does not exist/i.test(message)) {
      throw error;
    }
  }

  if (exists) {
    await ghApi(updateRefPath, { sha: baseOid, force: true }, "PATCH");
    console.log(`Updated refs/heads/${branch} → ${baseOid}`);
    return;
  }

  await ghApi(`/repos/${repo}/git/refs`, {
    ref: `refs/heads/${branch}`,
    sha: baseOid,
  });
  console.log(`Created refs/heads/${branch} → ${baseOid}`);
}

async function main() {
  const { branch, message, baseRef, repo } = parseArgs(process.argv.slice(2));

  git(["fetch", "origin", baseRef, "--prune"]);
  const baseOid = git(["rev-parse", `origin/${baseRef}`]);
  const { additions, deletions } = collectFileChanges(`origin/${baseRef}`);

  if (additions.length === 0 && deletions.length === 0) {
    console.error(
      `No file changes vs origin/${baseRef}; nothing to commit on ${branch}.`,
    );
    process.exit(1);
  }

  await ensureBranchAtOid(repo, branch, baseOid);

  const headline =
    message.length <= 256 ? message : `${message.slice(0, 253)}...`;

  const result = await ghGraphql(
    `mutation($input: CreateCommitOnBranchInput!) {
      createCommitOnBranch(input: $input) {
        commit { oid url }
      }
    }`,
    {
      input: {
        branch: {
          repositoryNameWithOwner: repo,
          branchName: branch,
        },
        message: { headline },
        fileChanges: { additions, deletions },
        expectedHeadOid: baseOid,
      },
    },
  );

  const commit =
    result &&
    typeof result === "object" &&
    "data" in result &&
    result.data &&
    typeof result.data === "object" &&
    "createCommitOnBranch" in result.data &&
    result.data.createCommitOnBranch &&
    typeof result.data.createCommitOnBranch === "object" &&
    "commit" in result.data.createCommitOnBranch
      ? result.data.createCommitOnBranch.commit
      : null;

  if (!commit || typeof commit !== "object" || !("oid" in commit)) {
    throw new Error(`Unexpected GraphQL response: ${JSON.stringify(result)}`);
  }

  console.log(
    `Created GitHub-signed commit ${commit.oid} on ${branch} (${additions.length} add, ${deletions.length} del)`,
  );
  if ("url" in commit && typeof commit.url === "string") {
    console.log(commit.url);
  }
  // Machine-readable for Actions steps (do not change prefix).
  console.log(`COMMIT_OID=${commit.oid}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
