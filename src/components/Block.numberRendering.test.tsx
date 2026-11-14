import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { blockProperty, node, pageByName } from "../document";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { endEdit, startEditing } from "../editorController";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { PageView } from "./Page";

installBlockEditorLifecycle();
afterEach(() => { endEdit("page-navigation"); vi.restoreAllMocks(); resetTabsToJournals(); });

// Read HTML ordinals so an equivalent renderer using ol.start also satisfies the test.
function visibleNumbers(root: HTMLElement) {
  return [...root.querySelectorAll<HTMLLIElement>("ol.md-list > li:not(.has-checkbox)")].map(item => {
    let number = (item.parentElement as HTMLOListElement).start;
    for (const sibling of item.parentElement!.children) {
      if (sibling.hasAttribute("value")) number = (sibling as HTMLLIElement).value;
      if (sibling === item) return number;
      number++;
    }
  });
}

describe.each<EditorKind>(["textarea", "cm6"])("routed number rendering (%s)", kind => {
  useEditorKind(kind);

  async function open(raw: string, format: "md" | "org" = "md") {
    const dto = { ...page("Numbers", [blk("numbers", raw)]), format, id: `pages/Numbers.${format}` };
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: dto.name, pageKind: "page" });
    const view = mount(() => <PageView />);
    await vi.waitFor(() => expect(view.root.querySelector('[data-block-id="numbers"]')).not.toBeNull());
    return view;
  }

  it.each(["md", "org"] as const)("keeps literal numbered continuation and its visible numbers (%s)", async format => {
    const view = await open("First line", format);
    try {
      startEditing("numbers", 0);
      const editor = editorIn(view.root);
      expect(editor.kind).toBe(kind);
      editor.focus(); editor.select(0);
      for (const ch of "1. ") editor.type(ch);
      expect(blockProperty("numbers", "logseq.order-list-type")).toBeNull();
      editor.select(editor.value().length);
      editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
      await vi.waitFor(() => expect(editor.selection()).toEqual([editor.value().length, editor.value().length]));
      editor.type("Second line");
      expect(editor.value()).toBe("1. First line\n2. Second line");
      expect(node("numbers")!.raw).toBe(editor.value());
      expect(pageByName("Numbers")!.roots).toEqual(["numbers"]);
      endEdit("page-navigation");
      const items = [...view.root.querySelectorAll<HTMLElement>("li.md-list-item")];
      expect(view.root.textContent).toContain("1. First line");
      expect(items.map(item => item.textContent?.trim())).toEqual(["Second line"]);
      expect(visibleNumbers(view.root)).toEqual([2]);
    } finally { view.dispose(); }
  });

  it.each([9, 10, 99, 100])("shows every code line number in view and editor (%i lines)", async count => {
    const body = Array.from({ length: count }, (_, i) => `const n${i} = ${i};`).join("\n");
    const view = await open(`\`\`\`js\n${body}\n\`\`\``);
    try {
      const labels = () => [...view.root.querySelectorAll(".code-gutter .calc-lineno")].map(el => el.textContent);
      expect(labels()).toEqual(Array.from({ length: count }, (_, i) => String(i + 1)));
      startEditing("numbers", 6);
      expect(editorIn(view.root).value()).toBe(body);
      expect(labels()).toEqual(Array.from({ length: count }, (_, i) => String(i + 1)));
    } finally { view.dispose(); }
  });

  it.each(["md", "org"] as const)("uses explicit numbers for nested and checkbox list items (%s)", async format => {
    const view = await open("Intro\n3. Third\n9. Ninth\n  5. Nested\n11. [ ] Task", format);
    try {
      expect(visibleNumbers(view.root)).toEqual([3, 9, 5]);
      expect(view.root.querySelector('[role="checkbox"]')?.getAttribute("aria-checked")).toBe("false");
    } finally { view.dispose(); }
  });

  it("creates the numbered code gutter through /code on a named page", async () => {
    const view = await open("");
    try {
      startEditing("numbers", 0);
      const editor = editorIn(view.root);
      editor.focus();
      for (const ch of "/code") editor.type(ch);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Code block"));
      editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("JavaScript"));
      editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
      const body = Array.from({ length: 10 }, (_, i) => `const n${i} = ${i};`).join("\n");
      editor.input(body, body.length, { inputType: "insertFromPaste", data: body });
      expect(view.root.querySelectorAll(".code-gutter .calc-lineno")).toHaveLength(10);
      expect(node("numbers")!.raw).toBe(`\`\`\`javascript\n${body}\n\`\`\``);
    } finally { view.dispose(); }
  });
});
