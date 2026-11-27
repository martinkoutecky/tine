import { beforeAll, expect, it } from "vitest";
import { blockRegions, initParser } from "../render/parse";
import { BUILTIN_HIDE, HIDE_ID, splitProps } from "./properties";
import { propertyEditorSession } from "./propertySession";

beforeAll(() => initParser());

it.each(["md", "org"] as const)("handwritten titleless history keeps the prior hidden identity (%s)", format => {
  const id = "11111111-1111-4111-8111-111111111111";
  const raw = format === "md" ? `plain\nid:: ${id}` : `plain\n:PROPERTIES:\n:id: ${id}\n:END:`;
  const visible = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 09:00:00]\n:END:";
  const session = propertyEditorSession();
  const parts = session.split(raw, BUILTIN_HIDE, format);
  const out = session.join(visible, parts.hidden, raw, BUILTIN_HIDE, format);
  expect(session.split(out, BUILTIN_HIDE, format).visible).toBe(visible);
  expect(blockRegions(out, format).id?.value).toBe(id);
  expect(blockRegions(out, format).drawers.map(d => d.name.toLowerCase())).toContain("logbook");
});

it.each(["md", "org"] as const)("a supplied LOGBOOK wins across subsequent edits (%s)", format => {
  const props = format === "org" ? ":PROPERTIES:\n:owner: Ada\n:END:" : "owner:: Ada";
  const old = `DOING x\n${props}\n:LOGBOOK:\nCLOCK: [2026-10-20 Tue 08:00:00]\n:END:`;
  const replacement = `DOING new\n${props}\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 09:00:00]\n:END:`;
  const session = propertyEditorSession();
  const parts = session.split(old, BUILTIN_HIDE, format);
  const joined = session.join(replacement, parts.hidden, old, BUILTIN_HIDE, format);
  expect(joined).toBe(replacement);
  const next = session.split(joined, BUILTIN_HIDE, format);
  expect(next.visible).toBe(replacement); // supplied content stays in this active editor
  expect(session.join(next.visible.replace("new", "newer"), next.hidden, joined, BUILTIN_HIDE, format)).toBe(replacement.replace("new", "newer"));
  expect(splitProps(old, HIDE_ID, format).visible).toBe(old); // whole-block copy retains clocks
  session.begin(); // exit/reentry uses the normal hidden projection
  expect(session.split(joined, BUILTIN_HIDE, format).visible).not.toContain("LOGBOOK");
});

it("typing preserves metadata order around a hidden Markdown drawer", () => {
  const old = "DOING x\ncollapsed:: true\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:\nDEADLINE: <2026-10-22 Thu>";
  const session = propertyEditorSession();
  const parts = session.split(old, BUILTIN_HIDE, "md");
  expect(session.join(parts.visible.replace("x", "é🙂"), parts.hidden, old, BUILTIN_HIDE, "md")).toBe(old.replace("x", "é🙂"));
});

// GH #975: a drawer in the buffer replacing the hidden history ("Edit clock
// entries", or one typed by hand) keeps the history's place and every hidden
// row's place; only the drawer's text changes.
it.each([
  ["md", "DOING x é🙂\nid:: 11111111-1111-4111-8111-111111111111\ncollapsed:: true\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]--[2026-10-21 Wed 09:00:00] =>  01:00:00\n:END:\nDEADLINE: <2026-10-22 Thu>\nnotes"],
  ["org", "DOING x é🙂\n:PROPERTIES:\n:id: 11111111-1111-4111-8111-111111111111\n:END:\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]--[2026-10-21 Wed 09:00:00] =>  01:00:00\n:END:\nnotes"],
] as const)("a drawer replacing the history keeps the hidden rows in place (%s)", (format, old) => {
  const session = propertyEditorSession();
  const parts = session.split(old, BUILTIN_HIDE, format);
  const drawer = old.slice(old.indexOf(":LOGBOOK:"), old.indexOf(":END:", old.indexOf(":LOGBOOK:")) + 5);
  const visible = parts.visible.replace("é🙂", `é🙂\n${drawer}`);
  expect(session.join(visible, parts.hidden, old, BUILTIN_HIDE, format)).toBe(old);
  const fixed = visible.replace("09:00:00] =>  01:00:00", "09:30:00] =>  01:30:00");
  const joined = session.join(fixed, parts.hidden, old, BUILTIN_HIDE, format);
  expect(joined).toBe(old.replace("09:00:00] =>  01:00:00", "09:30:00] =>  01:30:00"));
  const next = session.split(joined, BUILTIN_HIDE, format);
  expect(next.visible).toBe(fixed);
  expect(session.join(fixed.replace("x é", "y é"), next.hidden, joined, BUILTIN_HIDE, format)).toBe(joined.replace("x é", "y é"));
});

it.each(["md", "org"] as const)("showLogbook opens the drawer at its place, the caret on its first CLOCK row (%s)", format => {
  const props = format === "org" ? ":PROPERTIES:\n:collapsed: true\n:END:" : "collapsed:: true";
  const raw = `TODO x\n${props}\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:\nnotes`;
  const session = propertyEditorSession();
  const caret = session.showLogbook(raw, BUILTIN_HIDE, format);
  const parts = session.split(raw, BUILTIN_HIDE, format);
  expect(parts.visible).toBe("TODO x\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:\nnotes");
  expect(caret).toBe(parts.visible.indexOf("CLOCK:"));
  expect(session.join(parts.visible, parts.hidden, raw, BUILTIN_HIDE, format)).toBe(raw);
  session.begin();
  expect(session.split(raw, BUILTIN_HIDE, format).visible).toBe("TODO x\nnotes");
  expect(session.showLogbook("TODO x\nnotes", BUILTIN_HIDE, format)).toBeNull();
});
