import { afterEach, describe, expect, it } from "vitest";
import { EditorState, Transaction, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { editorIn, editorsIn, focusedEditor } from "./editorDriver";

const views: EditorView[] = [];
function mount(doc = "abc", extensions: Extension[] = []) {
  const host = document.createElement("div");
  host.dataset.blockId = "driver-fixture";
  document.body.append(host);
  const transactions: Transaction[] = [];
  const view = new EditorView({ parent: host, state: EditorState.create({ doc, extensions: [
    EditorView.editorAttributes.of({ "data-block-editor": "cm6" }),
    EditorView.updateListener.of(update => transactions.push(...update.transactions)), ...extensions,
  ] }) });
  views.push(view);
  return { view, driver: editorIn(host), transactions };
}
afterEach(() => {
  for (const view of views.splice(0)) { const host = view.dom.parentElement; view.destroy(); host?.remove(); }
});

describe("CM6 editor-driver public boundary", () => {
  it("discovers and focuses the marked editor and preserves UTF-16 selection direction", () => {
    const { driver, view, transactions } = mount("a😀bc");
    expect(driver.kind).toBe("cm6");
    expect(driver.element).toBe(view.contentDOM);
    expect(driver.blockId).toBe("driver-fixture");
    expect(editorsIn(document)).toHaveLength(1);
    driver.focus();
    expect(focusedEditor()?.element).toBe(driver.element);
    driver.select(1, 4, "backward");
    expect(driver.selection()).toEqual([1, 4]);
    expect(driver.direction()).toBe("backward");
    expect(transactions.at(-1)?.annotation(Transaction.userEvent)).toBeUndefined();
    driver.selectByUser(0, 3);
    expect(transactions.at(-1)?.annotation(Transaction.userEvent)).toBe("select");
    driver.select(99);
    expect(driver.selection()).toEqual([5, 5]);
    expect(driver.direction()).toBe("none");
  });

  it.each([
    ["insertText", false, "input.type"], ["insertFromPaste", false, "input.paste"],
    ["insertCompositionText", true, "input.type.compose"], ["deleteContentBackward", false, "delete.backward"],
    ["deleteContentForward", false, "delete.forward"], ["deleteByCut", false, "delete.cut"],
  ])("reports one minimal change for %s", (inputType, isComposing, userEvent) => {
    const { driver, transactions } = mount("prefix old suffix");
    driver.input("prefix new suffix", 10, { inputType, isComposing });
    expect(driver.value()).toBe("prefix new suffix");
    expect(driver.selection()).toEqual([10, 10]);
    expect(transactions).toHaveLength(1);
    expect(transactions[0].annotation(Transaction.userEvent)).toBe(userEvent);
    const changes: unknown[] = [];
    transactions[0].changes.iterChanges((from, to, _fromB, _toB, insert) => changes.push([from, to, insert.toString()]));
    expect(changes).toEqual([[7, 10, "new"]]);
  });

  it("offers typing to input handlers and uses the default transaction only when unhandled", () => {
    const calls: unknown[] = [];
    const { driver } = mount("abc", [EditorView.inputHandler.of((view, from, to, text, insert) => {
      calls.push([from, to, text]);
      if (text !== "handled") return false;
      view.dispatch(insert());
      return true;
    })]);
    driver.select(1, 2);
    driver.type("handled");
    expect(driver.value()).toBe("ahandledc");
    driver.type("!");
    expect(driver.value()).toBe("ahandled!c");
    expect(calls).toEqual([[1, 2, "handled"], [8, 8, "!"]]);
  });

  it.each(["textarea", "cm6"])("reports an edit before its plain platform input signal (%s)", kind => {
    const host = document.createElement("div");
    document.body.append(host);
    if (kind === "textarea") {
      const control = document.createElement("textarea");
      control.className = "block-editor";
      host.append(control);
    }
    const mounted = kind === "cm6" ? mount() : { driver: editorIn(host), transactions: [] };
    const { driver, transactions } = mounted;
    const seen: unknown[] = [];
    driver.element.addEventListener("input", event => {
      seen.push([event instanceof InputEvent, event.bubbles, driver.value(), driver.selection()]);
    });
    try {
      driver.input("a😀b", 3, { eventKind: "plain" });
      expect(seen).toEqual([[false, true, "a😀b", [3, 3]]]);
      if (kind === "cm6") expect(transactions.filter(tr => tr.docChanged)).toHaveLength(1);
    } finally { host.remove(); }
  });

  it("runs DOM keymaps and preserves supplied key and clipboard events", () => {
    let runs = 0;
    const { driver } = mount("abc", [keymap.of([{ key: "Enter", run: () => { runs++; return true; } }])]);
    const key = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    expect(driver.keydown(key)).toBe(key);
    expect(runs).toBe(1);
    expect(key.defaultPrevented).toBe(true);
    let received: Event | undefined;
    driver.element.addEventListener("paste", event => { received = event; event.preventDefault(); });
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    expect(driver.paste(paste)).toBe(paste);
    expect(received).toBe(paste);
  });

  it("reports semantic read-only state and refuses to invent an unreported draft buffer", () => {
    const { driver } = mount("abc", [EditorState.readOnly.of(true)]);
    expect(driver.isReadOnly()).toBe(true);
    expect(() => driver.replaceText("draft")).toThrow("not representable in CM6");
    expect(driver.value()).toBe("abc");
  });

  it("dispatches platform signals and composition to the content DOM", () => {
    const { driver, view, transactions } = mount();
    const seen: string[] = [];
    for (const type of ["input", "keyup", "contextmenu", "mouseup", "select", "compositionstart", "compositionend", "blur"]) {
      driver.element.addEventListener(type, () => seen.push(type));
    }
    driver.reportInput({}, "plain");
    driver.keyup({ key: "a" });
    driver.contextmenu(new MouseEvent("contextmenu"));
    driver.mouseup();
    driver.reportSelection(new Event("select"));
    expect(transactions.at(-1)?.annotation(Transaction.userEvent)).toBe("select");
    driver.compositionStart("あ");
    expect(view.compositionStarted).toBe(true);
    driver.compositionEnd("あ");
    driver.reportBlur();
    expect(seen).toEqual(["input", "keyup", "contextmenu", "mouseup", "select", "compositionstart", "compositionend", "blur"]);
  });
});
