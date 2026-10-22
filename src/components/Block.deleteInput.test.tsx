import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { deletePage } from "../document";
import { doc } from "../document/model";
import { loadRoutedPage } from "../document/workingSet";
import { bindTestHost, mailPage } from "../document/host/wiring.test.support";
import type { PageOperation } from "../document/host/protocol";
import { startEditing } from "../editorController";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { Block } from "./Block";

installBlockEditorLifecycle();
afterEach(() => vi.restoreAllMocks());

it("makes an open editor read-only during deletion and resumes editing after refusal", async () => {
  const dto = { ...page("Doomed", [blk("d1", "before")]), id: "pages/Doomed.md", rev: "r1" };
  loadRoutedPage(dto);
  const host = await bindTestHost();
  vi.spyOn(backend(), "pageOpen").mockImplementation(async (_session, id) => {
    queueMicrotask(() => host.deliver({ key: dto.id, page: mailPage(1, dto),
      answer: { id, version: 1, took: false, outcome: { kind: "applied" } } }));
    return { key: dto.id, baselineEntry: true };
  });
  startEditing("d1", 6);
  const view = mount(() => <Block id="d1" />);
  try {
    const textarea = view.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    expect(textarea.readOnly).toBe(false);
    let answer!: (operation: PageOperation) => void;
    const remove = vi.spyOn(backend(), "pageDelete").mockImplementation(() => new Promise((done) => { answer = done; }));
    const deleting = deletePage("Doomed", "page", "pages/Doomed.md");
    await vi.waitFor(() => expect(remove).toHaveBeenCalledOnce());
    expect(textarea.readOnly).toBe(true);
    expect(textarea.value).toBe("before");
    // Custom paste handlers can run even on a read-only textarea. The shared
    // store gate must refuse them and keep the visible buffer in step.
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: {
      getData: (type: string) => type === "text/plain" ? "blocked\npaste" : "", items: [], types: ["text/plain"],
    } });
    textarea.dispatchEvent(paste);
    await Promise.resolve();
    expect(textarea.value).toBe("before");
    expect(doc.byId.d1.raw).toBe("before");
    answer("refused");
    expect(await deleting).toBe(false);
    expect(textarea.readOnly).toBe(false);
    textarea.value = "after refusal";
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "l" }));
    expect(doc.byId.d1.raw).toBe("after refusal");
  } finally { view.dispose(); }
});
