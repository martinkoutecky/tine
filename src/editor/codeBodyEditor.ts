/** One textarea's reversible code view (GH #510). Selection in raw mixed text
 * enters a parser-owned body on collapsed native selection; boundary motion returns to raw
 * coordinates without committing. O(block text), no writes or syntax recognition.
 * Prefix/suffix stay frozen during body edits, including paste of fence-like text.
 * Whole-code wrappers retain their existing selection and navigation behavior.
 * `shown` projects the visible buffer; `join` splices the body into original raw
 * `source`, preserving hidden metadata too (source defaults to the visible raw).
 * `enter` takes a visible-source UTF-16 range and selects a body only if it contains the range.
 * `syncSelection` maps a collapsed native raw selection into that body.
 * `leaveOnKey` consumes boundary arrows/Backspace and restores raw coordinates.
 * `selection` supplies visible-source mixed-block coordinates to history (whole-code keeps
 * its existing body coordinates). No operation here commits or persists text. */
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
  const shown = createMemo(() => {
    if (!enabled()) return null;
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
    return codeBodyProjection(text, format());
  });
  const mixed = () => shown() !== null && codeBodyProjection(visibleRaw(), format()) === null;
  const enter = (caret: number, end = caret) => {
    if (!enabled()) return;
    const p = codeBodyProjection(visibleRaw(), format(), caret);
    setRegion(p && end <= p.open.length + p.body.length ? capture(p) : null);
  };
  const syncSelection = (ta: HTMLTextAreaElement) => {
    if (shown() || ta.selectionStart !== ta.selectionEnd) return;
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
    if (!p || !mixed() || e.ctrlKey || e.metaKey || e.altKey) return false;
    const start = ta.selectionStart, end = ta.selectionEnd;
    const before = (e.key === "ArrowLeft" || e.key === "Backspace") && start === 0
      || e.key === "ArrowUp" && !ta.value.slice(0, start).includes("\n");
    const after = e.key === "ArrowRight" && end === ta.value.length
      || e.key === "ArrowDown" && !ta.value.slice(end).includes("\n");
    if ((!before && !after) || (!e.shiftKey && start !== end)) return false;
    e.preventDefault();
    const anchor = p.open.length + (before ? end : start);
    const caret = before ? p.open.length - 1 : p.open.length + p.body.length + 1;
    setRegion(null);
    ta.value = visibleRaw();
    ta.setSelectionRange(e.shiftKey ? Math.min(anchor, caret) : caret,
      e.shiftKey ? Math.max(anchor, caret) : caret, before ? "backward" : "forward");
    return true;
  };
  return { shown, mixed, enter, syncSelection, leaveOnKey,
    join: (body: string) => {
      const p = shown();
      const captured = p && (region() ?? capture(p));
      // Raw wrappers also retain hidden metadata in its original position.
      if (!captured) return null;
      return codeBodyJoin(captured.source, body);
    },
    selection: (ta: HTMLTextAreaElement) => {
      const offset = mixed() ? shown()!.open.length : 0;
      return { start: ta.selectionStart + offset, end: ta.selectionEnd + offset };
    },
  };
}
