import { afterEach, describe, expect, it } from "vitest";
import { setDoc, loadedIdentityCollisions } from "./model";
const uuid = "6a55b643-0000-4000-8000-000000000000";
afterEach(() => { setDoc("byId", {}); setDoc("pages", []); });
describe("parser-owned loaded identities", () => {
  it("reserves runtime and accepted authored identities, not literal id lines", () => {
    setDoc("pages", [{ name: "p", kind: "page", title: "p", preBlock: null, roots: ["runtime"], format: "md", readOnly: false, guide: false }]);
    setDoc("byId", { runtime: { id: "runtime", raw: `text\n\`\`\`\nid:: ${uuid}\n\`\`\``, page: "p", parent: null, children: [], collapsed: false } });
    expect([...loadedIdentityCollisions([uuid, "runtime"])]).toEqual(["runtime"]);
    setDoc("byId", "runtime", "raw", `text\nid:: ${uuid}`);
    expect([...loadedIdentityCollisions([uuid.toUpperCase()])]).toEqual([uuid.toUpperCase()]);
  });
});

it("names the conservative reservation policy separately from external identity", () => {
  setDoc("pages", [{ name: "p", kind: "page", title: "p", preBlock: null, roots: ["runtime"], format: "md", readOnly: false, guide: false }]);
  setDoc("byId", { runtime: { id: "runtime", raw: `text\n:PROPERTIES:\n:id: ${uuid}\n:END:`, page: "p", parent: null, children: [], collapsed: false } });
  expect([...loadedIdentityCollisions([uuid])]).toEqual([uuid]);
});
