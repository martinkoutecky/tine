/** One textarea's reversible code view (GH #510). Selection in raw mixed text
 * enters a parser-owned body on collapsed native selection; boundary motion returns to raw
 * coordinates without committing. O(block text), no writes or syntax recognition.
 * Prefix/suffix stay frozen during body edits, including paste of fence-like text.
 * Code clicks select a body; block navigation retains the editable raw wrapper.
 * `shown` projects the visible buffer; `join` splices the body into original raw
 * `source`, preserving hidden metadata too (source defaults to the visible raw).
 * `enter` takes a visible-source UTF-16 range and selects a body only if it contains the range.
 * `syncSelection` maps a collapsed native raw selection into that body.
 * `leaveOnKey` consumes boundary arrows/Backspace and restores raw coordinates.
 * `selection` supplies visible-source coordinates to history in either view.
 * No operation here commits or persists text. */
import { createMemo, createSignal, type Accessor } from "solid-js";
import { codeFences } from "./fences";
import { codeBodyJoin, codeBodyProjection, type CodeBodyProjection } from "./codeFence";

export function createCodeBodyEditor(visibleRaw: Accessor<string>, format: Accessor<"md" | "org">, enabled: Accessor<boolean>, source: Accessor<string> = visibleRaw) {
  const capture = (p: CodeBodyProjection) => {
    const index = codeFences(visibleRaw(), format()).findIndex(f => f.openEnd === p.open.length);
    const original = codeFences(source(), format())[index];
    const sourceProjection = original && codeBodyProjection(source(), format(), original.openEnd);
    return sourceProjection ? { visible: p, source: sourceProjection } : null;
  };
  const [region, setRegion] = createSignal<ReturnType<typeof capture>>(null);
  const [rawMode, setRawMode] = createSignal(false);
  const shown = createMemo(() => {
    if (!enabled() || rawMode()) return null;
    const text = visibleRaw(), pair = region(), p = pair?.visible;
    const original = source();
    if (p && pair && original.startsWith(pair.source.open) && original.endsWith(pair.source.close) && text.startsWith(p.open) && text.endsWith(p.close) && text.length >= p.open.length + p.close.length) {
      let body = text.slice(p.open.length, text.length - p.close.length);
      // An originally empty fence has no separator in its suffix. Joining its
      // first nonempty body inserts one; keep that structural byte out of view.
      const separator = p.open.endsWith("\r\n") ? "\r\n" : "\n";
      if (!p.close.startsWith("\n") && !p.close.startsWith("\r\n") && body.endsWith(separator)) body = body.slice(0, -separator.length);
      return { ...p, body };
    }
    return null;
  });
  const mixed = () => shown() !== null && codeBodyProjection(visibleRaw(), format()) === null;
  const enter = (caret: number, end = caret) => {
    if (!enabled() || rawMode()) return;
    const p = codeBodyProjection(visibleRaw(), format(), caret);
    setRegion(p && end <= p.open.length + p.body.length ? capture(p) : null);
  };
  const syncSelection = (ta: HTMLTextAreaElement) => {
    // Whole-block raw editing stays raw as the caret moves through its source,
    // like OG's block editor. Mixed regions retain their inline edit behavior.
    if (shown() || codeBodyProjection(visibleRaw(), format()) || ta.selectionStart !== ta.selectionEnd) return;
    const caret = ta.selectionStart;
    enter(caret);
    const p = shown();
    if (p) {
      ta.value = p.body;
      ta.setSelectionRange(caret - p.open.length, caret - p.open.length);
    }
  };
  const leaveOnKey = (e: KeyboardEvent, ta: HTMLTextAreaElement) => {
    const p = shown();
    if (!p || e.ctrlKey || e.metaKey || e.altKey) return false;
    // Preserve the existing empty-card deletion door (GH #488).
    if (e.key === "Backspace" && ta.value === "" && !mixed()) return false;
    const start = ta.selectionStart, end = ta.selectionEnd;
    const before = (e.key === "ArrowLeft" || e.key === "Backspace") && start === 0
      || e.key === "ArrowUp" && !ta.value.slice(0, start).includes("\n");
    const after = e.key === "ArrowRight" && end === ta.value.length
      || e.key === "ArrowDown" && !ta.value.slice(end).includes("\n");
    // Escape is the editor's: it commits, then revealRaw keeps the source (GH #865).
    if ((!before && !after) || (!e.shiftKey && start !== end)) return false;
    e.preventDefault();
    const anchor = p.open.length + (before ? end : start);
    // Boundary arrows expose the adjacent fence.
    const caret = before ? p.open.length - 1 : p.open.length + p.body.length + 1;
    setRegion(null);
    ta.value = visibleRaw();
    ta.setSelectionRange(e.shiftKey ? Math.min(anchor, caret) : caret,
      e.shiftKey ? Math.max(anchor, caret) : caret, before ? "backward" : "forward");
    return true;
  };
  return { shown, mixed, enter, syncSelection, leaveOnKey,
    // OG extensions/code.cljs render!: Esc saves the code editor and calls
    // edit-block! with :max. Stay raw for this editing session so native
    // selection cannot immediately hide the delimiters again (GH #865).
    revealRaw: (ta: HTMLTextAreaElement) => {
      setRawMode(true);
      setRegion(null);
      ta.value = visibleRaw();
      ta.setSelectionRange(ta.value.length, ta.value.length);
      ta.focus();
    },
    join: (body: string) => {
      const p = shown();
      const captured = p && (region() ?? capture(p));
      // Raw wrappers also retain hidden metadata in its original position.
      if (!captured) return null;
      return codeBodyJoin(captured.source, body);
    },
    selection: (ta: HTMLTextAreaElement) => {
      const offset = shown()?.open.length ?? 0;
      return { start: ta.selectionStart + offset, end: ta.selectionEnd + offset };
    },
  };
}

/** After the fence-scaffold language picker closes, show only its body and
 * map the source selection into it, including input already committed there. */
export function reenterCodeBody(view: { enter: (start: number) => void }, scaffold: { open: string; body: string }, ref: HTMLTextAreaElement, autosize: () => void): void {
  const offset = (pos: number) => Math.max(0, Math.min(scaffold.body.length, pos - scaffold.open.length));
  const start = offset(ref.selectionStart), end = offset(ref.selectionEnd), direction = ref.selectionDirection;
  view.enter(scaffold.open.length);
  queueMicrotask(() => {
    ref.value = scaffold.body;
    ref.setSelectionRange(start, end, direction);
    autosize();
  });
}
