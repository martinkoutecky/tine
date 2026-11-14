/** One editor's reversible code view (GH #510, D-20261114-CS879). Explicit entry
 * selects a parser-owned body; boundary motion returns to raw
 * coordinates without committing. O(block text), no writes or syntax recognition.
 * Prefix/suffix stay frozen during body edits, including paste of fence-like text.
 * Code clicks select a body; block navigation retains the editable raw wrapper.
 * `shown` projects the visible buffer; `join` splices the body into original raw
 * `source`, preserving hidden metadata too (source defaults to the visible raw).
 * `enter` takes a visible-source UTF-16 range and selects a body only if it contains the range.
 * `leaveOnKey` consumes boundary arrows/Backspace and restores raw coordinates.
 * `leaveRange` supplies that same transition to editors with state-owned selections.
 * `selection` maps editor coordinates to source and carries the chosen body into restores, in O(1).
 * `revealRaw` alone calls the editor's guarded
 * commit before disabling projection and returning visible source, or null if
 * no code view is showing. Editors own display and caret placement. */
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
    // Source entry on an existing fence stays source, including language completion.
    // An initially plain editor can still create a scaffold and enter its body.
    if (!shown() && codeFences(visibleRaw(), format()).length) setRawMode(true);
  };
  const leaveRange = (e: KeyboardEvent, value: string, start: number, end: number) => {
    const p = shown();
    if (!p || e.ctrlKey || e.metaKey || e.altKey) return null;
    // Preserve the existing empty-card deletion door (GH #488).
    if (e.key === "Backspace" && value === "" && !mixed()) return null;
    const before = (e.key === "ArrowLeft" || e.key === "Backspace") && start === 0
      || e.key === "ArrowUp" && !value.slice(0, start).includes("\n");
    const after = e.key === "ArrowRight" && end === value.length
      || e.key === "ArrowDown" && !value.slice(end).includes("\n");
    if ((!before && !after) || (!e.shiftKey && start !== end)) return null;
    const anchor = p.open.length + (before ? end : start);
    // Boundary arrows expose the adjacent fence; Escape uses revealRaw below.
    const caret = before ? p.open.length - 1 : p.open.length + p.body.length + 1;
    setRegion(null);
    setRawMode(true);
    return { anchor: e.shiftKey ? anchor : caret, head: caret, direction: before ? "backward" as const : "forward" as const };
  };
  const leaveOnKey = (e: KeyboardEvent, ta: HTMLTextAreaElement) => {
    const range = leaveRange(e, ta.value, ta.selectionStart, ta.selectionEnd);
    if (!range) return false;
    e.preventDefault();
    ta.value = visibleRaw();
    ta.setSelectionRange(Math.min(range.anchor, range.head), Math.max(range.anchor, range.head), range.direction);
    return true;
  };
  return { shown, mixed, enter, leaveOnKey, leaveRange,
    // OG extensions/code.cljs render!: Esc saves the code editor and calls
    // edit-block! with :max. Stay raw for this editing session so native
    // selection cannot immediately hide the delimiters again (GH #865).
    revealRaw: (body: string, commit: (text: string) => void) => {
      if (!shown()) return null;
      commit(body);
      setRawMode(true);
      setRegion(null);
      return visibleRaw();
    },
    join: (body: string) => {
      const p = shown();
      const captured = p && (region() ?? capture(p));
      // Raw wrappers also retain hidden metadata in its original position.
      if (!captured) return null;
      return codeBodyJoin(captured.source, body);
    },
    selection: (start: number, end: number) => {
      const offset = shown()?.open.length ?? 0;
      return offset ? { start: start + offset, end: end + offset, codeBodyStart: offset } : { start, end };
    },
  };
}
