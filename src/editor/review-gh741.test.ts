import { beforeAll, expect, it } from "vitest";
import { initParser, blockRegions } from "../render/parse";
import { BUILTIN_HIDE, splitProps, joinProps } from "./properties";
import { propertyEditorSession } from "./propertySession";

beforeAll(() => initParser());
const drawer = ":LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:54:45]\n:END:";
const id = "11111111-1111-4111-8111-111111111111";

it("review clearing an Org task retains independent drawers", () => {
  const raw = `DOING x\n:PROPERTIES:\n:owner: Ada\n:id: ${id}\n:END:\n${drawer}\nBody`;
  const parts = splitProps(raw, BUILTIN_HIDE, "org");
  const out = joinProps("", parts.hidden, "org", raw);
  const regions = blockRegions(out, "org");
  expect(regions.id?.value).toBe(id);
  expect(regions.drawers.map(d => d.name.toLowerCase())).toContain("logbook");
  expect(regions.properties.filter(p => p.primary).map(p => [p.key, p.value])).toEqual([["id", id]]);
});

it.each(["md", "org"] as const)("review hand-typed drawer remains in active editor (%s)", format => {
  const session = propertyEditorSession();
  const raw = "plain";
  const parts = session.split(raw, BUILTIN_HIDE, format);
  const visible = `plain\n${drawer}`;
  const out = session.join(visible, parts.hidden, raw, BUILTIN_HIDE, format);
  expect(session.split(out, BUILTIN_HIDE, format).visible).toBe(visible);
  session.begin();
  expect(session.split(out, BUILTIN_HIDE, format).visible).toBe("plain");
});

it("review Org quote retains drawer ownership", () => {
  const raw = `DOING x\n${drawer}`;
  const parts = splitProps(raw, BUILTIN_HIDE, "org");
  const out = joinProps("> quoted", parts.hidden, "org", raw);
  expect(splitProps(out, BUILTIN_HIDE, "org").visible).toBe("> quoted");
  const next = joinProps("new", splitProps(out, BUILTIN_HIDE, "org").hidden, "org", out);
  expect(splitProps(next, BUILTIN_HIDE, "org")).toEqual({ visible: "new", hidden: drawer });
});
