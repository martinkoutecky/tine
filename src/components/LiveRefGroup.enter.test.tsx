import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { flushAll, node, resetStore } from "../document";
import { ensurePageLoaded } from "../document/workingSet";
import { pageToDto } from "../document/convert";
import { bindTestHost, submittedPages } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { editingId, endEdit } from "../editorController";
import { createMembershipResource, groupsContainEdit } from "../resultMembership";
import { initParser } from "../render/parse";
import { editorIn, editorsIn, maybeEditorIn } from "../tests/editorDriver";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

beforeAll(initParser);
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

// OG 6e7afa8eb editor.cljs outliner-insert-block!: Enter on a ref/query
// root inserts a child, which stays visible even when it is not a match.
// Model native order: Enter hands off BEFORE the debounced query publishes.
for (const surface of ["query", "ref"] as const) {
  for (const properties of [false, true]) {
    it.each(["middle matching", "middle nonmatching", "middle parent nonmatching", "end", "end folded"])(
      `${surface}, properties=${properties}: Enter at %s keeps typing through save and publication`, async where => {
        const host = await bindTestHost();
        answerOpensFromDocument(host);
        const submit = vi.spyOn(backend(), "pageSubmit");
        const text = where === "middle parent nonmatching" ? "Body tail [[Target]]"
          : "Body [[Target]] tail" + (where === "middle matching" ? " [[Target]]" : "");
        ensurePageLoaded({ id: "pages/Source.md", name: "Source", title: "Source", kind: "page", format: "md",
          pre_block: properties ? "tags:: [[Target]]\n" : null,
          blocks: [{ id: "body", raw: text, collapsed: where === "end folded", children: [] }],
        } as PageRead);
        const header: BlockDto[] = properties ? [{ id: "page-property:Page:source", raw: "tags:: [[Target]]",
          collapsed: false, children: [], page_property: true }] : [];
        const publish = () => {
          const matches = (blocks: BlockDto[]): BlockDto[] => blocks.flatMap(block =>
            block.raw.includes("[[Target]]") ? [block] : matches(block.children));
          const rows = [...header, ...matches(pageToDto("Source")!.blocks)];
          return [{ page: "Source", kind: "page", blocks: rows }] satisfies RefGroup[];
        };
        const [revision, setRevision] = createSignal(0);
        const read = vi.fn(async () => publish());
        const root = document.createElement("div");
        document.body.append(root);
        const dispose = render(() => {
          const [answer] = createMembershipResource(() => "Target", () => "Target", read,
            groupsContainEdit, () => true, surface === "query", revision);
          return <LiveRefGroup page="Source" kind="page" blocks={answer()?.[0]?.blocks ?? []} surface={surface} eager />;
        }, root);
        try {
          await vi.waitFor(() => expect(root.querySelector('[data-block-id="body"] .block-content')).not.toBeNull());
          const body = root.querySelector('[data-block-id="body"] .block-content')!;
          body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
          body.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
          const editor = await vi.waitFor(() => { expect(maybeEditorIn(root)).not.toBeNull(); return editorIn(root); });
          editor.focus();
          const offset = where.startsWith("end") ? text.length : text.indexOf("tail");
          editor.select(offset);
          editor.keydown({ key: "Enter" });
          const id = editingId()!;
          expect(id).not.toBe("body");
          // No publication has happened: the destination must already edit here.
          let next = await vi.waitFor(() => { expect(maybeEditorIn(root, id)).not.toBeNull(); return editorIn(root, id); });
          expect(node(id).parent).toBe("body");
          expect(document.activeElement).toBe(next.element);
          expect(next.value()).toBe(text.slice(offset).trimStart());
          next.type("continued ");
          expect(await flushAll()).toBe(true);
          setRevision(value => value + 1);
          next = await vi.waitFor(() => {
            const current = editorIn(root, id);
            expect(document.activeElement).toBe(current.element);
            expect(current.value()).toBe("continued " + text.slice(offset).trimStart());
            expect(current.selection()).toEqual([10, 10]);
            return current;
          });
          expect(editorsIn(root)).toHaveLength(1);
          expect(document.activeElement).toBe(next.element);
          next.type("after");
          expect(await flushAll()).toBe(true);
          expect(submittedPages(submit, "Source").at(-1)?.blocks[0].children[0].raw)
            .toBe("continued after" + text.slice(offset).trimStart());
          expect(read).toHaveBeenCalledTimes(1);
          expect(node("body").collapsed).toBe(where === "end folded");
          if (where.startsWith("end")) {
            next.select(next.value().length);
            next.keydown({ key: "Enter" });
            const sibling = editingId()!;
            expect(sibling).not.toBe(id);
            expect(node(sibling).parent).toBe("body");
            await vi.waitFor(() => expect(document.activeElement).toBe(editorIn(root, sibling).element));
            editorIn(root, sibling).type("sibling");
            expect(await flushAll()).toBe(true);
            expect(submittedPages(submit, "Source").at(-1)?.blocks[0].children[1].raw).toBe("sibling");
          }
          const readsWhileEditing = read.mock.calls.length;
          endEdit("blur");
          await vi.waitFor(() => expect(read.mock.calls.length).toBeGreaterThan(readsWhileEditing));
        } finally { dispose(); }
      });
  }
}
