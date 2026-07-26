import type { BodyAttribute, KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  filterBodyAttributesByTraitType,
  findBodyAttribute,
  listBodyAttributes,
} from "./attributes.js";

function makeKnowledgeEntry(
  overrides: Partial<KnowledgeEntry> & Pick<KnowledgeEntry, "entry_id">,
): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

const affiliationGuild: BodyAttribute = {
  trait_type: "affiliation",
  value: "Guild",
};

const affiliationCrown: BodyAttribute = {
  trait_type: "affiliation",
  value: "Crown",
};

const roleProtagonist: BodyAttribute = {
  trait_type: "role",
  value: "protagonist",
};

describe("listBodyAttributes", () => {
  it("returns [] when input is null or undefined", () => {
    expect(listBodyAttributes(undefined)).toEqual([]);
    expect(listBodyAttributes(null)).toEqual([]);
  });

  it("returns [] when attributes are omitted or empty", () => {
    expect(listBodyAttributes({ summary: "Only summary" })).toEqual([]);
    expect(listBodyAttributes({ attributes: [] })).toEqual([]);
  });

  it("returns [] when entry body or attributes are absent", () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_1" });
    expect(listBodyAttributes(entry)).toEqual([]);
  });

  it("reads attributes from a full KnowledgeEntry", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_1",
      body: {
        attributes: [affiliationGuild, roleProtagonist],
      },
    });

    expect(listBodyAttributes(entry)).toEqual([affiliationGuild, roleProtagonist]);
  });

  it("returns all valid traits in order and skips malformed elements", () => {
    const body = {
      attributes: [
        affiliationGuild,
        { trait_type: "affiliation", value: "Crown" },
        { trait_type: "", value: "empty-type" },
        { trait_type: "role", value: { nested: true } },
        null,
        "not-an-object",
        { trait_type: "level", value: 3, display_type: "number" },
        { value: "missing-type" },
      ],
    };

    expect(listBodyAttributes(body)).toEqual([
      affiliationGuild,
      affiliationCrown,
      { trait_type: "level", value: 3, display_type: "number" },
    ]);
  });

  it("returns [] when attributes is not an array", () => {
    expect(
      listBodyAttributes({
        attributes: "invalid" as unknown as BodyAttribute[],
      }),
    ).toEqual([]);
  });
});

describe("filterBodyAttributesByTraitType", () => {
  const body = {
    attributes: [affiliationGuild, affiliationCrown, roleProtagonist],
  };

  it("returns all matches in order for duplicate trait_type values", () => {
    expect(filterBodyAttributesByTraitType(body, "affiliation")).toEqual([
      affiliationGuild,
      affiliationCrown,
    ]);
  });

  it("returns [] when trait_type has no matches", () => {
    expect(filterBodyAttributesByTraitType(body, "missing")).toEqual([]);
    expect(filterBodyAttributesByTraitType(undefined, "affiliation")).toEqual(
      [],
    );
  });

  it("matches trait_type with exact case-sensitive equality", () => {
    expect(filterBodyAttributesByTraitType(body, "Affiliation")).toEqual([]);
    expect(filterBodyAttributesByTraitType(body, "affiliation")).toEqual([
      affiliationGuild,
      affiliationCrown,
    ]);
  });
});

describe("findBodyAttribute", () => {
  const body = {
    attributes: [affiliationGuild, affiliationCrown, roleProtagonist],
  };

  it("returns the first matching trait in array order", () => {
    expect(findBodyAttribute(body, "affiliation")).toEqual(affiliationGuild);
    expect(findBodyAttribute(body, "role")).toEqual(roleProtagonist);
  });

  it("returns undefined when no match or input is absent", () => {
    expect(findBodyAttribute(body, "missing")).toBeUndefined();
    expect(findBodyAttribute(undefined, "role")).toBeUndefined();
    expect(findBodyAttribute({ attributes: [] }, "role")).toBeUndefined();
  });
});
