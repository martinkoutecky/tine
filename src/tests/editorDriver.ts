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

export type EditorKind = "textarea";
export type SelectionDirection = "forward" | "backward" | "none";

export interface EditorInput {
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
  reportInput(init?: InputEventInit, eventKind?: "input" | "plain"): void;
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
  paste(clip: EditorPaste): Event;
  compositionStart(data?: string): void;
  compositionEnd(data?: string): void;
}

const EDITOR_SELECTOR = "textarea.block-editor";

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
    const event = new InputEvent("input", { bubbles: true, inputType, data });
    ta.dispatchEvent(init.isComposing ? withProps(event, { isComposing: true }) : event);
  };
  return {
    kind: "textarea",
    element: ta,
    get blockId() {
      return blockIdOf(ta);
    },
    value: () => ta.value,
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
      ta.dispatchEvent(eventKind === "plain" ? new Event("input", init) : new InputEvent("input", init));
    },
    mouseup: (init = {}) => { ta.dispatchEvent(new MouseEvent("mouseup", init)); },
    input,
    type: (text) => {
      const start = ta.selectionStart;
      const value = ta.value.slice(0, start) + text + ta.value.slice(ta.selectionEnd);
      input(value, start + text.length, { inputType: "insertText", data: text });
    },
    keydown: (init, extra = {}) => {
      const event = withProps(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }), extra);
      ta.dispatchEvent(event);
      return event;
    },
    keyup: (init) => {
      const event = new KeyboardEvent("keyup", { bubbles: true, cancelable: true, ...init });
      ta.dispatchEvent(event);
      return event;
    },
    paste: (clip) => {
      const event = withProps(new Event("paste", { bubbles: true, cancelable: true }), {
        clipboardData: clipboardData(clip),
      });
      ta.dispatchEvent(event);
      return event;
    },
    compositionStart: (data = "") => {
      ta.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data }));
    },
    compositionEnd: (data = "") => {
      ta.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data }));
    },
  };
}

function driverFor(el: Element): EditorDriver {
  if (el instanceof HTMLTextAreaElement) return textareaDriver(el);
  throw new Error(`editorDriver: no backend for ${el.tagName.toLowerCase()}.${[...el.classList].join(".")}`);
}

/** Every mounted block editor under `root`, in document order. */
export function editorsIn(root: ParentNode = document): EditorDriver[] {
  return [...root.querySelectorAll(EDITOR_SELECTOR)].map(driverFor);
}

/** The block editor under `root` (optionally of block `blockId`), or null. */
export function maybeEditorIn(root: ParentNode = document, blockId?: string): EditorDriver | null {
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
  return active?.matches(EDITOR_SELECTOR) ? driverFor(active) : null;
}
