import { beforeAll, afterEach, expect, it, vi } from "vitest";
import { propertyEditorSession } from "./propertySession";
import { clearSeededFacets, facetsFromDto, seedFacets } from "../render/facets";
import { loadSingle, resetStore } from "../document/workingSet";
import * as parser from "../render/parse";

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
  expect(read.mock.calls.length, "I-12/I-25: loaded identity absence reuses lsdoc facts; exemplar propertySession.ts").toBe(0);
});

it("retains structural ownership for possible ids, edits, formats and older DTOs", () => {
  const session = propertyEditorSession();
  const raw = "Loaded row";
  seedFacets(raw, "md", facetsFromDto({ has_id: false }));
  const otherFormat = vi.spyOn(parser, "blockRegions");
  session.identity(raw, "org");
  expect(otherFormat.mock.calls.length).toBe(1);
  otherFormat.mockRestore();
  expect(session.identity(raw + "\nid:: authored", "md").value).toBe("authored");
  for (const [format, text] of [
    ["md", "Row\nid:: first\nid:: later"],
    ["org", "Row\n:PROPERTIES:\n:ID: owned\n:END:\nbody\n:PROPERTIES:\n:ID: body\n:END:"],
  ] as const) {
    const expected = parser.blockRegions(text, format).id?.value ?? null;
    seedFacets(text, format, facetsFromDto({ has_id: true }));
    expect(session.identity(text, format).value).toBe(expected);
  }
  expect(session.identity("Older DTO\nid:: older", "md").value).toBe("older");
  clearSeededFacets();
  const read = vi.spyOn(parser, "blockRegions");
  session.identity(raw, "md");
  expect(read).toHaveBeenCalled();
});
