import { afterEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parseThemeManifest } from "./manifest";

let scratch = "";

afterEach(() => {
  if (scratch) fs.rmSync(scratch, { recursive: true, force: true });
  scratch = "";
});

function manifestFor(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schemaVersion: 1,
    id: "page.tine.theme.test",
    name: "Test",
    version: "1.0.0",
    apiVersion: "0.2",
    description: "A checker fixture.",
    author: "Tine",
    license: "MIT",
    source: "https://example.invalid/theme",
    modes: { light: { "--ls-primary-background-color": "#fff" } },
    screenshots: [],
    ...overrides,
  };
}

function check(overrides: Record<string, unknown> = {}) {
  if (scratch) fs.rmSync(scratch, { recursive: true, force: true });
  scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tine-theme-check-"));
  const manifest = manifestFor(overrides);
  fs.writeFileSync(path.join(scratch, "theme.json"), JSON.stringify(manifest));
  const result = spawnSync(process.execPath, ["scripts/tine-theme.mjs", "check", scratch, "--json"], {
    cwd: process.cwd(),
    encoding: "utf8",
  });
  return JSON.parse(result.stdout) as { status: "passed" | "failed"; errors: Array<{ code: string }> };
}

describe("theme package checker", () => {
  it("accepts API 0.2 host presentation presets", () => {
    expect(check({ presentation: {
      contentTypography: "editorial-serif",
      journalHeader: "editorial",
      todayTaskSummary: "compact",
    } }).status).toBe("passed");
  });

  it("keeps API 0.1 color-only and rejects fields the runtime rejects", () => {
    expect(check({ apiVersion: "0.1" }).status).toBe("passed");
    expect(check({ apiVersion: "0.1", presentation: { journalHeader: "editorial" } }).errors)
      .toEqual(expect.arrayContaining([expect.objectContaining({ code: "theme.presentation-api" })]));
    expect(check({ presentation: { rawCss: "body { display: none }" } }).errors)
      .toEqual(expect.arrayContaining([expect.objectContaining({ code: "theme.presentation-field" })]));
    expect(check({ arbitraryRootField: true }).errors)
      .toEqual(expect.arrayContaining([expect.objectContaining({ code: "theme.field" })]));
  });
});

// GH #410: the standalone registry checker and the parser the app actually
// installs with had drifted — the checker asked only for source/revision/authors,
// so a manifest naming an unsupported `ecosystem`, or carrying an unknown
// provenance field, passed intake and then failed on the user's machine. The
// contract below is one-directional on purpose: the checker is allowed to be
// STRICTER than the runtime (the SPDX allow-list and the dotted id are registry
// intake policy, not runtime rules), never laxer.
const PORTED_FROM_FIXTURES: Array<{ what: string; portedFrom: unknown }> = [
  { what: "a complete port record", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "an ecosystem the runtime does not know", portedFrom: {
    ecosystem: "notion", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "a relationship the runtime does not know", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "inspired-by",
  } },
  { what: "no relationship at all", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"],
  } },
  { what: "no upstream name", portedFrom: {
    ecosystem: "logseq", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "no upstream license", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "a field the runtime rejects outright", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
    notes: "ported by hand",
  } },
  { what: "an author list that is empty", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: [], relationship: "behavioral-port",
  } },
  { what: "an author list past the runtime's limit", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT",
    authors: Array.from({ length: 33 }, (_, index) => `Author ${index}`), relationship: "behavioral-port",
  } },
  { what: "an author that is not text", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: [{ name: "Upstream Author" }], relationship: "behavioral-port",
  } },
  { what: "a name past the runtime's length limit", portedFrom: {
    ecosystem: "logseq", name: "x".repeat(121), source: "https://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "a revision carrying a control character", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "https://example.invalid/upstream",
    revision: "a1b2\u0007c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "an upstream source that is not https", portedFrom: {
    ecosystem: "logseq", name: "Dev Theme", source: "http://example.invalid/upstream",
    revision: "a1b2c3d", license: "MIT", authors: ["Upstream Author"], relationship: "behavioral-port",
  } },
  { what: "provenance that is not an object", portedFrom: "ported from Logseq" },
];

const COMPLETE_PORT = PORTED_FROM_FIXTURES[0]!.portedFrom as Record<string, unknown>;

function installs(portedFrom: unknown): boolean {
  try {
    parseThemeManifest(manifestFor({ portedFrom }));
    return true;
  } catch {
    return false;
  }
}

describe("theme port provenance (GH #410)", () => {
  it("certifies a complete port record", () => {
    expect(check({ portedFrom: COMPLETE_PORT }).status).toBe("passed");
  });

  // The vocabulary has a third authority: the schema theme authors write
  // against. It was already in step with the runtime when the checker was not,
  // so pin all three together rather than leaving the next drift to chance.
  it("keeps the published schema, the checker and the app on one vocabulary", () => {
    const schema = JSON.parse(fs.readFileSync("plugin-sdk/schema/theme.schema.json", "utf8")) as {
      $defs: { portedFrom: { required: string[]; properties: Record<string, { enum?: string[] }> } };
    };
    const def = schema.$defs.portedFrom;

    // Every field the schema demands is a field the app demands.
    for (const field of def.required) {
      const withoutField = { ...COMPLETE_PORT };
      delete withoutField[field];
      expect(installs(withoutField), `dropping ${field}`).toBe(false);
      expect(check({ portedFrom: withoutField }).status, `dropping ${field}`).toBe("failed");
    }
    expect(installs({ ...COMPLETE_PORT, notes: "hand ported" })).toBe(false);

    // Every value the schema offers is a value the app and the checker take.
    for (const ecosystem of def.properties.ecosystem!.enum!) {
      expect(installs({ ...COMPLETE_PORT, ecosystem }), ecosystem).toBe(true);
      expect(check({ portedFrom: { ...COMPLETE_PORT, ecosystem } }).status, ecosystem).toBe("passed");
    }
    for (const relationship of def.properties.relationship!.enum!) {
      expect(installs({ ...COMPLETE_PORT, relationship }), relationship).toBe(true);
      expect(check({ portedFrom: { ...COMPLETE_PORT, relationship } }).status, relationship).toBe("passed");
    }
  });

  it("never certifies a port record the app would refuse to install", () => {
    for (const fixture of PORTED_FROM_FIXTURES) {
      const certified = check({ portedFrom: fixture.portedFrom }).status === "passed";
      // The checker may refuse more than the app; it may never certify less.
      expect(certified && !installs(fixture.portedFrom), fixture.what).toBe(false);
    }
  });
});
