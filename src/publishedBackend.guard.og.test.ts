import { readFileSync } from "node:fs";
import ts from "typescript";
import { expect, it } from "vitest";
import { PUBLISHED_ABSENT_METHODS, PUBLISHED_ANSWERED_METHODS,
  PUBLISHED_CONSTANT_METHODS, PUBLISHED_REFUSED_METHODS, publishedBackend } from "./publishedBackend";

// A newly added native Backend method needs an explicit published answer,
// quiet browser constant, absence, or refusal. The Proxy cannot make that
// decision because an accidentally refused read leaves a blank published page.
it("classifies every og Backend member and implements its browser reads", () => {
  const file = ts.createSourceFile("backend.ts", readFileSync(new URL("./backend.ts", import.meta.url), "utf8"), ts.ScriptTarget.Latest, true);
  const backend = file.statements.find((item): item is ts.InterfaceDeclaration =>
    ts.isInterfaceDeclaration(item) && item.name.text === "Backend");
  expect(backend).toBeDefined();
  const declared = new Set(backend!.members.map((member) => member.name?.getText(file)));
  const classes = [PUBLISHED_ANSWERED_METHODS, PUBLISHED_CONSTANT_METHODS,
    PUBLISHED_REFUSED_METHODS, PUBLISHED_ABSENT_METHODS].map((items) => new Set<string>(items));
  const missing = [...declared].filter((name) => name && !classes.some((group) => group.has(name)));
  expect(missing, "classify each new Backend method in publishedBackend.ts").toEqual([]);
  const duplicate = [...declared].filter((name) => name && classes.filter((group) => group.has(name)).length > 1);
  expect(duplicate).toEqual([]);
  const api = publishedBackend(async () => { throw new Error("snapshot read in classification"); }) as unknown as Record<string, unknown>;
  for (const name of [...declared]) {
    if (!name) continue;
    if (classes[0].has(name) || classes[1].has(name)) {
      expect(Object.prototype.hasOwnProperty.call(api, name), name).toBe(true);
      expect(typeof api[name], name).toBe("function");
    } else if (classes[3].has(name)) {
      expect(api[name], name).toBeUndefined();
    }
  }
});
