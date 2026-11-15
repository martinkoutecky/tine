import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { node, resetStore } from "../document";
import { ensurePageLoaded } from "../document/workingSet";
import { bindTestHost, submittedPages } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { editingId, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { LiveRefGroup } from "./LiveRefGroup";
import { Block } from "./Block";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { useEditorKind } from "../tests/editorKind";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";

useEditorKind("cm6");
beforeAll(initParser);
afterEach(() => { endEdit("page-navigation"); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

function click(element: Element) {
  element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
}

it("clicking the mounted query-result editor autosaves the source while retaining focus and the query", async () => {
  const host = await bindTestHost();
  answerOpensFromDocument(host);
  const block = { id: "task", raw: "TODO my task", collapsed: false, children: [] };
  const query = "{{tine-query @block AND task = 'TODO'}}";
  ensurePageLoaded({ id: "pages/Tasks.md", name: "Tasks", title: "Tasks", kind: "page", format: "md", pre_block: null, blocks: [block] });
  ensurePageLoaded({ id: "pages/Queries.md", name: "Queries", title: "Queries", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "query", raw: query, collapsed: false, children: [] }] });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{ page: "Tasks", kind: "page", blocks: [block] }]));
  const submit = vi.spyOn(backend(), "pageSubmit");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="query" />, root);
  try {
    const body = await vi.waitFor(() => { const el = root.querySelector('.live-ref-group [data-block-id="task"] .block-content'); expect(el).not.toBeNull(); return el!; });
    click(body);
    const editor = await vi.waitFor(() => { expect(maybeEditorIn(root, "task")).not.toBeNull(); return editorIn(root, "task"); });
    editor.focus();
    click(editor.element.querySelector(".cm-line") ?? editor.element);
    // Reacquire as native typing does: a stolen editor must not hide behind a
    // detached handle that can no longer receive the user's input.
    const active = editorIn(root);
    active.input("CANC my task");
    await vi.waitFor(() => expect(submittedPages(submit, "Tasks").at(-1)?.blocks[0].raw).toBe("CANC my task"));
    expect(editingId()).toBe("task");
    expect(document.activeElement).toBe(editor.element);
    expect(node("query").raw).toBe(query);
    expect(submittedPages(submit, "Queries")).toHaveLength(0);
    expect(submit).toHaveBeenCalledOnce();
  } finally { dispose(); }
});

it.each(["query", "ref", "embed", "preview", "sidebar"] as const)("%s live block autosaves while focused", async location => {
  const surface = location === "sidebar" ? "ref" : location;
  const host = await bindTestHost();
  answerOpensFromDocument(host);
  const block = { id: "task", raw: "TODO my task", collapsed: false, children: [] };
  ensurePageLoaded({ id: "pages/Tasks.md", name: "Tasks", title: "Tasks", kind: "page", format: "md", pre_block: null, blocks: [block] });
  adoptResults(block, captureResultsTicket());
  const submit = vi.spyOn(backend(), "pageSubmit");
  const root = document.createElement("div"); root.className = location === "sidebar" ? "right-sidebar" : "main-content"; document.body.append(root);
  const dispose = render(() => <LiveRefGroup page="Tasks" kind="page" blocks={[block]} surface={surface} eager />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector('[data-find-source-mounted="true"]')).not.toBeNull());
    const body = await vi.waitFor(() => { const el = root.querySelector('[data-block-id="task"] .block-content'); expect(el).not.toBeNull(); return el!; });
    click(body);
    const editor = await vi.waitFor(() => { expect(maybeEditorIn(root)).not.toBeNull(); return editorIn(root); });
    editor.focus();
    click(editor.element.querySelector(".cm-line") ?? editor.element);
    editor.input("CANC my task");
    expect(node("task").raw).toBe("CANC my task");
    await vi.waitFor(() => expect(submittedPages(submit, "Tasks").at(-1)?.blocks[0].raw).toBe("CANC my task"));
    expect(document.activeElement).toBe(editor.element);
    expect(editor.value()).toBe("CANC my task");
    expect(submit).toHaveBeenCalledOnce();
  } finally { dispose(); }
});
