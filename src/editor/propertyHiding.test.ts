import { beforeAll, afterEach, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";
import { BUILTIN_HIDE, SHEET_CELL_HIDE, editorHidePolicy, splitProps, joinProps, isEditorEmpty } from "./properties";
import { propertyEditorSession } from "./propertySession";

beforeAll(() => initParser());
afterEach(() => setGraphMeta(null));

it.each(["heading", "background-color", "hl-page", "card-last-score", "custom-id", "done"])("hides %s and preserves its bytes through edits and empty text", key => {
  for (const format of ["md", "org"] as const) {
    const line = format === "md" ? `${key}:: value  \t` : `:${key}: value  \t`;
    const raw = format === "md" ? `Body\n${line}` : `Body\n:PROPERTIES:\n${line}\n:END:`;
    const parts = splitProps(raw, BUILTIN_HIDE, format);
    expect(parts.visible).toBe("Body");
    expect(parts.hidden).toBe(line);
    for (const text of ["Revised", ""]) {
      const joined = joinProps(text, parts.hidden, format, raw);
      expect(joined).toContain(line);
      expect(splitProps(joined, BUILTIN_HIDE, format).visible).toBe(text);
    }
  }
});

it("preserves hidden Org CRLF rows when the textarea supplies LF text", () => {
  const raw = "Body\r\n:PROPERTIES:\r\n:heading: true  \t\r\n:owner: Ada\r\n:END:";
  const parts = splitProps(raw, BUILTIN_HIDE, "org");
  for (const text of ["Revised", "", "Revised\n:PROPERTIES:\n:owner: Ada\n:END:"]) {
    const joined = joinProps(text, parts.hidden, "org", raw);
    expect(joined, text).toContain(":heading: true  \t\r\n");
    expect(splitProps(joined, BUILTIN_HIDE, "org").hidden).toBe(parts.hidden);
  }
});

it("uses the graph's configured hidden properties and follows config changes", () => {
  const raw = "Body\nsecret:: original  \t\nowner:: Ada";
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  const parts = splitProps(raw, BUILTIN_HIDE);
  expect(parts.visible).toBe("Body\nowner:: Ada");
  expect(joinProps("Revised", parts.hidden, "md", raw)).toContain("secret:: original  \t");
  setGraphMeta(null);
  expect(splitProps(raw, BUILTIN_HIDE).visible).toBe(raw);
});

it("keeps editable built-ins and literal fenced metadata visible", () => {
  const raw = "Body\ntitle:: Title\nicon:: X\nfilters:: {}\nlogseq.color:: red\nlogseq.table.compact:: true\n```\nheading:: literal\n```";
  expect(splitProps(raw, BUILTIN_HIDE).visible).toBe(raw);
});

it("keeps handwritten hidden properties in an active buffer until reentry and follows new config", () => {
  const session = propertyEditorSession();
  const typed = "Body\nheading:: true";
  const raw = session.join(typed, "", "Body", BUILTIN_HIDE, "md");
  expect(session.split(raw, BUILTIN_HIDE, "md").visible).toBe(typed);
  session.begin();
  expect(session.split(raw, BUILTIN_HIDE, "md").visible).toBe("Body");
  const user = "Body\nsecret:: opaque";
  expect(session.split(user, BUILTIN_HIDE, "md").visible).toBe(user);
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  expect(session.split(user, BUILTIN_HIDE, "md").visible).toBe("Body");
});

it("does not discard newly hidden metadata by reusing an apparently empty block", () => {
  expect(isEditorEmpty("heading:: true", BUILTIN_HIDE, "md")).toBe(false);
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  expect(isEditorEmpty("secret:: opaque", BUILTIN_HIDE, "md")).toBe(false);
  expect(isEditorEmpty("collapsed:: true", BUILTIN_HIDE, "md")).toBe(true);
  setGraphMeta({ block_hidden_properties: ["tine.custom"] } as GraphMeta);
  expect(isEditorEmpty("tine.custom:: opaque", BUILTIN_HIDE, "md")).toBe(false);
});


// Review R2 (2026-10-24): a sheet-cell projection elsewhere must not replace the editor's
// configured policy, whose identity keys the session cache (a typed configured key would
// otherwise vanish from the textarea mid-word).
it("the editor's configured policy survives a sheet-cell projection", () => {
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  const editor = editorHidePolicy(BUILTIN_HIDE);
  splitProps("x\nsecret:: s", SHEET_CELL_HIDE, "md");
  expect(editorHidePolicy(SHEET_CELL_HIDE)).not.toBe(editor);
  expect(editorHidePolicy(BUILTIN_HIDE)).toBe(editor);
  const session = propertyEditorSession();
  const typed = "Body\nsecret:: tr";
  const raw = session.join(typed, "", "Body", BUILTIN_HIDE, "md");
  splitProps(raw, SHEET_CELL_HIDE, "md");
  expect(session.split(raw, BUILTIN_HIDE, "md").visible).toBe(typed);
});
