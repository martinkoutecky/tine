import { focusedEditorCommandBridge } from "../editorCommandBridge";
import { activeElement, documentOf } from "../windowRealm";

// GH #767: which events a keyboard sends for one Enter press in a block editor. Console-only, like
// `window.__tineWatcherLatency` (GH #337): `await __tineImeTrace()`, press Enter once in a block, and the
// promise resolves with the lines to paste into an issue. Nothing listens until it is called, and the
// listeners are removed when it resolves. A line holds event names, key codes, flags and lengths, never text.

const EVENTS = ["keydown", "keyup", "beforeinput", "input", "compositionstart", "compositionupdate", "compositionend"];
const CONTEXT_EVENTS = ["compositionstart", "compositionend", "textupdate"];
/** How long recording continues after the first Enter, so the keyboard's follow-up events are included. */
export const IME_TRACE_TAIL_MS = 500;

type Content = HTMLElement & { editContext?: EventTarget | null };
const focusedContent = () => activeElement()?.closest?.(".cm-content") as Content | null ?? null;

/** The focused editor's document length against the text the page shows. */
function lengths(): string {
  const content = focusedContent();
  if (!content) return "editor=none";
  const live = focusedEditorCommandBridge()?.textCaret?.();
  return `doc=${live ? live.text.length : "?"} dom=${content.textContent?.length ?? 0}`
    + ` domLines=${content.querySelectorAll(".cm-line").length}`;
}

function describe(event: Event, on: string, start: number): string {
  const parts = [`+${Math.round(performance.now() - start)}ms`, on, event.type];
  const e = event as Partial<KeyboardEvent & InputEvent & CompositionEvent>
    & { text?: string; updateRangeStart?: number; updateRangeEnd?: number };
  // A printable key would name the character typed; only named keys (Enter, Backspace, Unidentified) are kept.
  if (event.type.startsWith("key")) parts.push(`key=${(e.key?.length ?? 0) > 1 ? e.key : "char"}`, `keyCode=${e.keyCode}`, `isComposing=${e.isComposing}`);
  else if (event.type.endsWith("input")) parts.push(`inputType=${e.inputType}`, `isComposing=${e.isComposing}`, `dataLength=${e.data?.length ?? 0}`);
  else if (event.type === "textupdate") parts.push(`range=${e.updateRangeStart}-${e.updateRangeEnd}`, `textLength=${e.text?.length ?? 0}`);
  else parts.push(`dataLength=${e.data?.length ?? 0}`);
  parts.push(lengths());
  return parts.join(" ");
}

/** Record the events of one Enter press in the focused block editor. Resolves with the trace text. */
export function recordImeTrace(): Promise<string> {
  const start = performance.now();
  const content = focusedContent();
  const doc = documentOf(content ?? undefined), context = content?.editContext;
  const lines = [`EditContext=${!!doc.defaultView && "EditContext" in doc.defaultView} focused editor=${!!content}`
    + ` editContext=${!!context} ${lengths()}`];
  return new Promise(resolve => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onDocument = (event: Event) => {
      lines.push(describe(event, "element", start));
      const input = (event as Partial<InputEvent>).inputType ?? "";
      if (event.type === "keydown" && (event as KeyboardEvent).key === "Enter"
        || event.type === "beforeinput" && /^insert(Paragraph|LineBreak)$/.test(input)) timer ??= setTimeout(done, IME_TRACE_TAIL_MS);
    };
    const onContext = (event: Event) => { lines.push(describe(event, "editContext", start)); };
    const done = () => {
      for (const type of EVENTS) doc.removeEventListener(type, onDocument, true);
      for (const type of CONTEXT_EVENTS) context?.removeEventListener(type, onContext);
      lines.push(`after ${lengths()}`);
      resolve(lines.join("\n"));
    };
    for (const type of EVENTS) doc.addEventListener(type, onDocument, true);
    for (const type of CONTEXT_EVENTS) context?.addEventListener(type, onContext);
  });
}
