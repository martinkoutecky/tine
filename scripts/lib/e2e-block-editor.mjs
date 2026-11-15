// Native journeys share one editor boundary. Typing stays on WebDriver's native
// key/clipboard path; DOM inspection and selection never replace note content.
export const BLOCK_EDITOR = ':is(textarea.block-editor, [data-block-editor="cm6"] .cm-content[contenteditable="true"])';

/** The mounted input in one page/query/sidebar scope; no graph IO. */
export const editorSelector = (scope = "") => `${scope} ${BLOCK_EDITOR}`.trim();

function driverFor(element) {
  if (!element) return null;
  const classic = element instanceof HTMLTextAreaElement;
  // This is the pinned DOM-to-view link used by EditorView.findFromDOM.
  const view = classic ? null : element.cmTile?.root?.view;
  if (!classic && !view) throw new Error("Mounted CodeMirror input has no EditorView");
  return {
    element,
    get length() { return classic ? element.value.length : view.state.doc.length; },
    get value() { return classic ? element.value : view.state.doc.toString(); },
    get start() { return classic ? element.selectionStart : view.state.selection.main.from; },
    get end() { return classic ? element.selectionEnd : view.state.selection.main.to; },
    get focused() { return document.activeElement === element; },
    get blockId() { return element.closest("[data-block-id]")?.getAttribute("data-block-id") ?? null; },
    scrollElement: classic ? element : element.closest(".cm-scroller"),
    kind: classic ? "textarea" : "cm6",
    caretRect() {
      if (!classic) return view.coordsAtPos(view.state.selection.main.head);
      const cs = getComputedStyle(element);
      const mirror = document.createElement("div");
      for (const p of ["boxSizing", "width", "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "borderTopWidth", "borderRightWidth", "borderBottomWidth", "borderLeftWidth", "fontFamily", "fontSize", "fontWeight", "fontStyle", "letterSpacing", "lineHeight", "wordSpacing", "tabSize", "whiteSpace", "wordBreak", "overflowWrap"]) mirror.style[p] = cs[p];
      mirror.style.position = "absolute"; mirror.style.left = "-9999px"; mirror.style.visibility = "hidden";
      document.body.appendChild(mirror);
      try {
        mirror.textContent = "M";
        const pad = parseFloat(cs.paddingTop) || 0;
        const height = mirror.clientHeight - pad - (parseFloat(cs.paddingBottom) || 0);
        const at = element.selectionEnd;
        mirror.textContent = "";
        mirror.appendChild(document.createTextNode(element.value.slice(0, at)));
        const marker = document.createElement("span"); marker.textContent = element.value.slice(at, at + 1) || "\u200b";
        mirror.appendChild(marker);
        mirror.appendChild(document.createTextNode(element.value.slice(at + 1)));
        const row = Math.max(0, Math.round((marker.offsetTop - pad) / height));
        const rect = element.getBoundingClientRect();
        const top = rect.top + element.clientTop + pad + row * height - element.scrollTop;
        return { top, bottom: top + height };
      } finally { mirror.remove(); }
    },
    select(start, end = start) {
      element.focus();
      if (classic) {
        element.setSelectionRange(start, end);
        element.dispatchEvent(new Event("select", { bubbles: true }));
      } else {
        view.dispatch({ selection: { anchor: start, head: end }, userEvent: "select" });
      }
    },
  };
}

/** Inspect the scoped editor with a self-contained WebDriver callback. The
 * callback receives a live read/selection driver or null. It must return only
 * serializable observations. One request; reads cost O(buffer length), selection
 * O(1). Only trusted harness function source is serialized, never note content. */
async function executeEditorCallback(browser, scope, callback, args, all) {
  const query = all ? "Array.from(document.querySelectorAll(selector), driverFor)" : "driverFor(document.querySelector(selector))";
  const execute = new Function("selector", "...args",
    `const driverFor = ${driverFor.toString()}; return (${callback.toString()})(${query}, ...args);`);
  return browser.execute(execute, editorSelector(scope), ...args);
}

export async function withEditor(browser, scope, callback, ...args) {
  return executeEditorCallback(browser, scope, callback, args, false);
}

/** Inspect every mounted editor in a scope in one request. O(mounted editors
 * plus buffers read by the callback); never discovers hidden blocks or pages. */
export async function withEditors(browser, scope, callback, ...args) {
  return executeEditorCallback(browser, scope, callback, args, true);
}

/** Read one mounted editor's complete buffer and UTF-16 selection in one
 * WebDriver request, O(buffer length). Returns null when no editor is mounted.
 * CM's pinned DOM-to-view link is the same link used by EditorView.findFromDOM;
 * reading state also covers lines outside CM's virtualized DOM viewport. */
export async function editorState(browser, scope = "") {
  return withEditor(browser, scope, editor => {
    if (!editor) return null;
    return {
      value: editor.value, start: editor.start, end: editor.end,
      focused: editor.focused, blockId: editor.blockId, kind: editor.kind,
    };
  });
}

/** Read the complete buffer; a missing editor is an explicit harness failure. */
export async function editorText(browser, scope = "") {
  const state = await editorState(browser, scope);
  if (!state) throw new Error(`No mounted block editor in ${scope || "document"}`);
  return state.value;
}

/** Select UTF-16 offsets in one mounted editor, without changing its buffer.
 * Like a user selection, this reports the selection to the editor toolbar.
 * One WebDriver request, O(1); invalid/missing surfaces fail the journey. */
export async function selectEditor(browser, start, end = start, scope = "") {
  await withEditor(browser, scope, (editor, start, end) => {
    if (!editor) throw new Error("No mounted block editor to select");
    editor.select(start, end);
  }, start, end);
}

/** Commit by moving focus out of the scoped editor, in one WebDriver request. */
export async function blurEditor(browser, scope = "") {
  await browser.execute(selector => document.querySelector(selector)?.blur(), editorSelector(scope));
}

/** Replace the focused editor through the native plain-text clipboard. This
 * intentionally sets the scratch session's clipboard; clipboard journeys own
 * their payloads instead. No direct buffer mutation or application test hook. */
export async function pasteEditorText(browser, text) {
  const empty = await withEditor(browser, "", editor => {
    if (!editor?.focused) throw new Error("No focused block editor to replace");
    return editor.length === 0;
  });
  const result = await browser.executeAsync((text, done) => {
    window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|write_text", { text })
      .then(() => done({ ok: true }), error => done({ error: String(error) }));
  }, text);
  if (!result.ok) throw new Error(`Cannot prepare native clipboard: ${result.error}`);
  // On an empty block, Mod+A selects the outline block and leaves editing.
  if (!empty) await browser.keys(["Control", "a"]);
  await browser.keys(["Control", "v"]);
}
