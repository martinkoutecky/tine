import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "../components/Block";
import { startEditing, endEdit } from "../editorController";
import { loadSingle, resetStore } from "../document/workingSet";
import { doc } from "../document/model";

afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

function mount(raw: string, format: "md" | "org") {
  loadSingle({ name: "OwnProperties", title: "OwnProperties", kind: "page", format, pre_block: null,
    blocks: [{ id: "target", raw, collapsed: false, children: [] }] });
  startEditing("target", 0);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Block id="target" />, root);
  return { dispose, ta: root.querySelector("textarea.block-editor") as HTMLTextAreaElement };
}

it("the Org editor hides the own ID after CLOSED planning", () => {
  const { ta, dispose } = mount("DONE Task\nCLOSED: [2026-09-30 Wed 09:00]\n:PROPERTIES:\n:id: own-id\n:END:\nbody", "org");
  try {
    expect(ta.value).toBe("DONE Task\nCLOSED: [2026-09-30 Wed 09:00]\nbody");
    expect(doc.byId.target.raw).toContain(":id: own-id");
  } finally { dispose(); }
});

it.each([
  ["md", "prose\nid:: own-id", 400],
  ["org", "Title\n:PROPERTIES:\n:id: own-id\n:END:", 400],
  ["org", "Title\n:PROPERTIES:\n:owner: Jane\n:id: own-id\n:END:", 200],
] as const)("typing cost with hidden own metadata: %s / %s", (format, raw, misses) => {
  const { ta, dispose } = mount(raw, format);
  const w = window as unknown as { __tineBench?: boolean; __tineParseStats?: { calls: number; hits: number; misses: number } };
  w.__tineBench = true;
  w.__tineParseStats = { calls: 0, hits: 0, misses: 0 };
  try {
    for (let i = 0; i < 200; i++) {
      ta.value = `x${ta.value}`;
      ta.setSelectionRange(1, 1);
      ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "x" }));
    }
    console.info(`OG-R4B ${format} hidden metadata typing: ${JSON.stringify(w.__tineParseStats)}`);
    expect(w.__tineParseStats.misses).toBe(misses);
    expect(doc.byId.target.raw).toContain(format === "org" ? ":id: own-id" : "id:: own-id");
  } finally { dispose(); w.__tineBench = false; }
});
