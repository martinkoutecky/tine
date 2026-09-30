import { beforeAll, afterEach, expect, it, vi } from "vitest";
import { propertyEditorSession } from "./propertySession";
import { clearSeededFacets, facetsFromDto, seedFacets } from "../render/facets";
import { loadSingle, resetStore } from "../document/workingSet";
import * as parser from "../render/parse";
import { blockExternalId, existingBlockId, resolveBlockRef } from "../document/edits/identity";
import { blockRefCount } from "../blockRefCounts";
import { readFileSync } from "node:fs";

beforeAll(() => parser.initParser());
afterEach(() => { resetStore(); clearSeededFacets(); vi.restoreAllMocks(); });

it("uses the parser-owned absence fact for loaded rows without parsing every offscreen block", () => {
  const read = vi.spyOn(parser, "blockRegions");
  const blocks = Array.from({ length: 2000 }, (_, i) => ({
    id: `loaded-${i}`, raw: `Loaded row ${i}`, has_id: false, collapsed: false, children: [],
  }));
  loadSingle({ name: "Loaded", title: "Loaded", kind: "page", pre_block: null, format: "md", blocks });
  for (const { raw } of blocks) {
    expect(propertyEditorSession().identity(raw, "md")).toEqual({ raw, format: "md", value: null });
  }
  for (const { id, raw } of blocks) {
    expect(existingBlockId(raw, "md")).toBeNull();
    expect(blockExternalId(id)).toBe(id);
    expect(blockRefCount(id)).toBe(0);
  }
  expect(resolveBlockRef({ uuid: "loaded-0", page: "Loaded", pageKind: "page" })).toBe("loaded-0");
  expect(read.mock.calls.length, "I-12/I-25: every identity consumer reuses loaded lsdoc absence through existingBlockId; exemplar document/edits/identity.ts").toBe(0);
});

it("retains structural ownership for possible ids, edits, formats and older DTOs", () => {
  const session = propertyEditorSession();
  const raw = "Loaded row";
  seedFacets(raw, "md", facetsFromDto({ has_id: false }));
  const otherFormat = vi.spyOn(parser, "blockRegions");
  session.identity(raw, "org");
  expect(existingBlockId(raw, "org")).toBeNull();
  expect(otherFormat.mock.calls.length).toBe(2);
  otherFormat.mockRestore();
  expect(session.identity(raw + "\nid:: authored", "md").value).toBe("authored");
  expect(existingBlockId(raw + "\nid:: authored", "md")).toBe("authored");
  for (const [format, text] of [
    ["md", "Row\nid:: first\nid:: later"],
    ["org", "Row\n:PROPERTIES:\n:ID: owned\n:END:\nbody\n:PROPERTIES:\n:ID: body\n:END:"],
  ] as const) {
    const expected = parser.blockRegions(text, format).id?.value ?? null;
    seedFacets(text, format, facetsFromDto({ has_id: true }));
    expect(session.identity(text, format).value).toBe(expected);
    expect(existingBlockId(text, format)).toBe(expected);
  }
  expect(session.identity("Older DTO\nid:: older", "md").value).toBe("older");
  expect(existingBlockId("Older DTO\nid:: older", "md")).toBe("older");
  clearSeededFacets();
  const read = vi.spyOn(parser, "blockRegions");
  session.identity(raw, "md");
  existingBlockId(raw, "md");
  expect(read).toHaveBeenCalled();
});

it("keeps exact editor-owned identity authoritative and rejects mismatched buffers", () => {
  seedFacets("visible", "org", facetsFromDto({ has_id: false }));
  expect(existingBlockId("visible", "org", { raw: "visible", format: "org", value: " hidden " })).toBe("hidden");
  expect(() => existingBlockId("visible", "org", { raw: "other", format: "org", value: null })).toThrow("different buffer");
  expect(() => existingBlockId("visible", "org", { raw: "visible", format: "md", value: null })).toThrow("different buffer");
});

it("routes editor and badge identity through the shared answerer", () => {
  const editor = readFileSync(new URL("./propertySession.ts", import.meta.url), "utf8");
  const badges = readFileSync(new URL("../blockRefCounts.ts", import.meta.url), "utf8");
  expect(editor, "I-12: ordinary editor identity uses existingBlockId; exemplar document/edits/identity.ts").toContain("value: existingBlockId(raw, format)");
  expect(editor).not.toContain("knownIdentityAbsent");
  expect(badges, "I-12: badges use blockExternalId, which delegates to existingBlockId").toContain("blockExternalId(id)");
});
