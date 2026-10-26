/**
 * Editor-neutral test driver for the block editor (CM6 stage 1, D-20261025-BW).
 *
 * Component tests talk to "the editor" through this interface instead of a
 * `textarea.block-editor`, so the same test can run against the textarea editor
 * and an alternative editor (the CodeMirror 6 experiment) by adding a backend
 * here, never by editing the tests. Offsets are UTF-16 code units, as both the
 * textarea and CodeMirror report them.
 *
 * Rules for tests: reach the editor only through `editorIn`/`editorsIn`; state
 * an edit as its outcome (`input`) or as the user's action (`type`, `keydown`,
 * `paste`, composition); assert through `value()`/`selection()`. `element` is
 * for focus/containment checks and event listeners, not for `.value` or
 * selection APIs. A test of a textarea-only quirk (e.g. WebView selection
 * getters) may use `element` as a textarea, says so in a comment, and guards
 * on `kind === "textarea"`.
 */

import { EditorSelection, EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

export type EditorKind = "textarea" | "cm6";
export type SelectionDirection = "forward" | "backward" | "none";

export interface EditorInput {
  /** Preserve platforms that report a plain Event rather than an InputEvent. */
  eventKind?: "input" | "plain";
  /** `InputEvent.inputType`; defaults to "insertText". */
  inputType?: string;
  /** `InputEvent.data`; defaults to the character before the caret for text insertion. */
  data?: string | null;
  /** Report the input as part of an IME composition. */
  isComposing?: boolean;
}

export interface EditorPaste {
  text?: string;
  html?: string;
  files?: File[];
}

export interface EditorDriver {
  readonly kind: EditorKind;
  /** The element that receives focus and the editor's DOM events. */
  readonly element: HTMLElement;
  /** `data-block-id` of the enclosing block row, if any. */
  readonly blockId: string | null;
  value(): string;
  /** Whether edits are currently refused by this editor. */
  isReadOnly(): boolean;
  /** `[start, end]`, start ≤ end. */
  selection(): [number, number];
  direction(): SelectionDirection;
  /** Sets the selection programmatically (no user-selection signal). */
  select(start: number, end?: number, direction?: SelectionDirection): void;
  /** The user selected `[start, end]`: sets it and fires the editor's user-selection signal. */
  selectByUser(start: number, end?: number, direction?: SelectionDirection): void;
  focus(): void;
  blur(): void;
  /** Reports a platform blur, including one delivered after the editor was detached. */
  reportBlur(): void;
  /** Replaces the browser's draft without reporting input yet (e.g. before a key commits it). */
  replaceText(value: string): void;
  /** Reports input for the current draft and selection, preserving plain platform input signals too. */
  reportInput(init?: InputEventInit | Event, eventKind?: "input" | "plain"): void;
  /** Releases a pointer over the editor, including the user-selection notification. */
  mouseup(init?: MouseEventInit): void;
  /**
   * The user's edit left the editor holding `value` with the caret at `caret`
   * (default: end), reported as one input event. The common "set value, set
   * caret, dispatch input" step of the textarea tests.
   */
  input(value: string, caret?: number, init?: EditorInput): void;
  /** Types `text` over the current selection: one `insertText` input. */
  type(text: string): void;
  /** Dispatches one keydown (cancelable, bubbling); returns it for `defaultPrevented`. */
  keydown(init: KeyboardEventInit & { key: string }, extra?: { isComposing?: boolean; keyCode?: number }): KeyboardEvent;
  keyup(init: KeyboardEventInit & { key: string }): KeyboardEvent;
  /** Dispatches a paste carrying `clip`; returns it for `defaultPrevented`. */
  paste(clip: EditorPaste | Event): Event;
  /** Open the editor's context menu with the platform event's original flags. */
  contextmenu(event: MouseEvent): Event;
  /** Report the platform's selection signal without replacing the current range. */
  reportSelection(event: Event): void;
  compositionStart(data?: string | CompositionEvent): void;
  compositionEnd(data?: string | CompositionEvent): void;
}

const EDITOR_SELECTOR = 'textarea.block-editor, [data-block-editor="cm6"]';

function blockIdOf(el: Element): string | null {
  return el.closest<HTMLElement>("[data-block-id]")?.dataset.blockId ?? null;
}

function withProps<E extends Event>(event: E, props: Record<string, unknown>): E {
  for (const [key, value] of Object.entries(props)) {
    if (value !== undefined) Object.defineProperty(event, key, { value });
  }
  return event;
}

function clipboardData(clip: EditorPaste) {
  const files = clip.files ?? [];
  const types = [
    ...(clip.text !== undefined ? ["text/plain"] : []),
    ...(clip.html ? ["text/html"] : []),
    ...(files.length ? ["Files"] : []),
  ];
  return {
    files,
    items: files.map((file) => ({ kind: "file", type: file.type, getAsFile: () => file })),
    types,
    getData: (type: string) => (type === "text/html" ? clip.html ?? "" : type === "text/plain" ? clip.text ?? "" : ""),
  };
}

function textareaDriver(ta: HTMLTextAreaElement): EditorDriver {
  const input = (value: string, caret = value.length, init: EditorInput = {}) => {
    ta.value = value;
    ta.setSelectionRange(caret, caret);
    const inputType = init.inputType ?? "insertText";
    const data = init.data !== undefined ? init.data : inputType.startsWith("insert") ? value[caret - 1] ?? null : null;
    const event = init.eventKind === "plain" ? new Event("input", { bubbles: true })
      : new InputEvent("input", { bubbles: true, inputType, data });
    ta.dispatchEvent(init.isComposing ? withProps(event, { isComposing: true }) : event);
  };
  return {
    kind: "textarea",
    element: ta,
    get blockId() {
      return blockIdOf(ta);
    },
    value: () => ta.value,
    isReadOnly: () => ta.readOnly,
    selection: () => [ta.selectionStart, ta.selectionEnd],
    direction: () => ta.selectionDirection,
    select: (start, end = start, direction) => ta.setSelectionRange(start, end, direction),
    selectByUser: (start, end = start, direction) => {
      ta.setSelectionRange(start, end, direction);
      ta.dispatchEvent(new Event("select", { bubbles: true }));
    },
    focus: () => ta.focus(),
    // A focused textarea fires its own blur; an unfocused one gets the synthetic
    // event the tests used to dispatch by hand. Never both.
    blur: () => (document.activeElement === ta ? ta.blur() : void ta.dispatchEvent(new FocusEvent("blur"))),
    reportBlur: () => { ta.dispatchEvent(new FocusEvent("blur")); },
    replaceText: (value) => { ta.value = value; },
    reportInput: (init = {}, eventKind = "input") => {
      ta.dispatchEvent(init instanceof Event ? init : eventKind === "plain" ? new Event("input", init) : new InputEvent("input", init));
    },
    mouseup: (init = {}) => { ta.dispatchEvent(new MouseEvent("mouseup", init)); },
    input,
    type: (text) => {
      const start = ta.selectionStart;
      const value = ta.value.slice(0, start) + text + ta.value.slice(ta.selectionEnd);
      input(value, start + text.length, { inputType: "insertText", data: text });
    },
    keydown: (init, extra = {}) => {
      const event = init instanceof KeyboardEvent ? init : withProps(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }), extra);
      ta.dispatchEvent(event);
      return event;
    },
    keyup: (init) => {
      const event = init instanceof KeyboardEvent ? init : new KeyboardEvent("keyup", { bubbles: true, cancelable: true, ...init });
      ta.dispatchEvent(event);
      return event;
    },
    paste: (clip) => {
      const event = clip instanceof Event ? clip : withProps(new Event("paste", { bubbles: true, cancelable: true }), {
        clipboardData: clipboardData(clip),
      });
      ta.dispatchEvent(event);
      return event;
    },
    contextmenu: (event) => { ta.dispatchEvent(event); return event; },
    reportSelection: (event) => { ta.dispatchEvent(event); },
    compositionStart: (data = "") => {
      ta.dispatchEvent(data instanceof CompositionEvent ? data : new CompositionEvent("compositionstart", { bubbles: true, data }));
    },
    compositionEnd: (data = "") => {
      ta.dispatchEvent(data instanceof CompositionEvent ? data : new CompositionEvent("compositionend", { bubbles: true, data }));
    },
  };
}

/** Map browser input intents to CM's public transaction user-event vocabulary. */
function cm6UserEvent(init: EditorInput): string {
  if (init.isComposing || /Composition/.test(init.inputType ?? "")) return "input.type.compose";
  const type = init.inputType ?? "insertText";
  if (/Paste/.test(type)) return "input.paste";
  if (/Drop/.test(type)) return type.startsWith("delete") ? "delete.cut" : "input.drop";
  if (type === "deleteByCut") return "delete.cut";
  if (type.startsWith("delete")) return /Forward$/.test(type) ? "delete.forward" : "delete.backward";
  if (type === "historyUndo") return "undo";
  if (type === "historyRedo") return "redo";
  return type.startsWith("insert") ? "input.type" : "input";
}

function cm6Driver(root: Element): EditorDriver {
  const view = EditorView.findFromDOM(root as HTMLElement);
  if (!view) throw new Error("editorDriver: CM6 marker has no mounted EditorView");
  const element = view.contentDOM;
  const select = (start: number, end = start, direction?: SelectionDirection, userEvent?: string) => {
    const limit = (n: number) => Math.max(0, Math.min(n, view.state.doc.length));
    end = limit(end);
    start = Math.min(limit(start), end);
    view.dispatch({ selection: direction === "backward"
      ? EditorSelection.single(end, start) : EditorSelection.single(start, end), userEvent });
  };
  const input = (value: string, caret = value.length, init: EditorInput = {}) => {
    const old = view.state.doc.toString();
    let from = 0, oldEnd = old.length, newEnd = value.length;
    while (from < oldEnd && from < newEnd && old[from] === value[from]) from++;
    while (oldEnd > from && newEnd > from && old[oldEnd - 1] === value[newEnd - 1]) { oldEnd--; newEnd--; }
    // One minimal replacement preserves mapped ranges/history, including IME edits.
    view.dispatch({ changes: { from, to: oldEnd, insert: value.slice(from, newEnd) },
      selection: { anchor: Math.max(0, Math.min(caret, value.length)) }, userEvent: cm6UserEvent(init) });
    // CM owns the buffer in state; retain the platform signal after reporting
    // the edit through its transaction, without inventing a detached draft.
    if (init.eventKind === "plain") element.dispatchEvent(new Event("input", { bubbles: true }));
  };
  return {
    kind: "cm6", element,
    get blockId() { return blockIdOf(element); },
    value: () => view.state.doc.toString(),
    // CM exposes semantic read-only state separately from DOM editability.
    isReadOnly: () => view.state.facet(EditorState.readOnly) || !view.state.facet(EditorView.editable),
    selection: () => [view.state.selection.main.from, view.state.selection.main.to],
    // CM stores anchor/head, with no independent textarea "none" direction.
    direction: () => { const s = view.state.selection.main; return s.empty ? "none" : s.anchor > s.head ? "backward" : "forward"; },
    select,
    selectByUser: (start, end, direction) => select(start, end, direction, "select"),
    focus: () => view.focus(),
    blur: () => element.ownerDocument.activeElement === element ? element.blur() : void element.dispatchEvent(new FocusEvent("blur")),
    // Preserve even detached platform notifications; application blur guards decide ownership.
    reportBlur: () => { element.dispatchEvent(new FocusEvent("blur")); },
    // CM has no separate browser-value buffer. A transaction would notify the
    // application immediately, changing the ordering these fixtures exercise.
    replaceText: () => { throw new Error("not representable in CM6: an unreported textarea draft separate from editor state"); },
    // A platform input notification alone does not edit CM state. Real DOM
    // mutations are reconciled by CM's observer; input(value) models the edit.
    reportInput: (init = {}, eventKind = "input") => {
      element.dispatchEvent(init instanceof Event ? init : eventKind === "plain" ? new Event("input", init) : new InputEvent("input", init));
    },
    // Pointer release retains flags; CM reconciles native DOM selections itself.
    mouseup: (init = {}) => { element.dispatchEvent(new MouseEvent("mouseup", init)); },
    input,
    type: (text) => {
      const { from, to } = view.state.selection.main;
      const insert = () => view.state.update({ changes: { from, to, insert: text },
        selection: { anchor: from + text.length }, userEvent: cm6UserEvent({ isComposing: view.compositionStarted }), scrollIntoView: true });
      if (!view.state.facet(EditorView.inputHandler).some(handler => handler(view, from, to, text, insert))) view.dispatch(insert());
    },
    keydown: (init, extra = {}) => {
      const event = init instanceof KeyboardEvent ? init : withProps(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }), extra);
      element.dispatchEvent(event);
      return event;
    },
    keyup: (init) => {
      const event = init instanceof KeyboardEvent ? init : new KeyboardEvent("keyup", { bubbles: true, cancelable: true, ...init });
      element.dispatchEvent(event);
      return event;
    },
    // Dispatch through CM's DOM paste pipeline, including app clipboard handlers.
    paste: (clip) => {
      const event = clip instanceof Event ? clip : withProps(new Event("paste", { bubbles: true, cancelable: true }), { clipboardData: clipboardData(clip) });
      element.dispatchEvent(event);
      return event;
    },
    contextmenu: (event) => { element.dispatchEvent(event); return event; },
    // CM selection is state-owned. Signal the existing range as a user
    // selection transaction as well as preserving the original platform event.
    reportSelection: (event) => {
      view.dispatch({ selection: view.state.selection, userEvent: "select" });
      element.dispatchEvent(event);
    },
    compositionStart: (data = "") => { element.dispatchEvent(data instanceof CompositionEvent ? data : new CompositionEvent("compositionstart", { bubbles: true, data })); },
    compositionEnd: (data = "") => { element.dispatchEvent(data instanceof CompositionEvent ? data : new CompositionEvent("compositionend", { bubbles: true, data })); },
  };
}

function driverFor(el: Element): EditorDriver {
  // Workspace-window fixtures live in another document/realm.
  if (el.tagName.toLowerCase() === "textarea") return textareaDriver(el as HTMLTextAreaElement);
  if (el.matches('[data-block-editor="cm6"]')) return cm6Driver(el);
  throw new Error(`editorDriver: no backend for ${el.tagName.toLowerCase()}.${[...el.classList].join(".")}`);
}

/** Every mounted block editor under `root`, in document order. */
export function editorsIn(root: ParentNode | null = document): EditorDriver[] {
  return [...(root?.querySelectorAll(EDITOR_SELECTOR) ?? [])].map(driverFor);
}

/** The block editor under `root` (optionally of block `blockId`), or null. */
export function maybeEditorIn(root: ParentNode | null = document, blockId?: string): EditorDriver | null {
  return editorsIn(root).find((editor) => blockId === undefined || editor.blockId === blockId) ?? null;
}

/** The block editor under `root` (optionally of block `blockId`); throws when none is mounted. */
export function editorIn(root: ParentNode = document, blockId?: string): EditorDriver {
  const editor = maybeEditorIn(root, blockId);
  if (!editor) throw new Error(`editorDriver: no block editor${blockId ? ` for ${blockId}` : ""} is mounted`);
  return editor;
}

/** The driver for the focused block editor, or null when focus is elsewhere. */
export function focusedEditor(): EditorDriver | null {
  const active = document.activeElement;
  const root = active?.closest(EDITOR_SELECTOR);
  return root ? driverFor(root) : null;
}
