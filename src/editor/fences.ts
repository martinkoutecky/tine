/** CommonMark-style fenced-code state shared by every editor raw-line scan. */
export interface FenceState {
  char: "`" | "~";
  length: number;
}

export interface FenceTransition {
  opens: boolean;
  closes: boolean;
  next: FenceState | null;
}

const FENCE_RUN = /^\s*(`{3,}|~{3,})/;

/** Advance fenced-code state for one line.
 *
 * A closer must use the opener's character and be at least as long. A shorter
 * run is literal code content (not a close followed by a new open).
 */
export function transitionFence(state: FenceState | null, line: string): FenceTransition {
  const match = FENCE_RUN.exec(line);
  if (!match) return { opens: false, closes: false, next: state };
  const run = match[1];
  const marker = { char: run[0] as FenceState["char"], length: run.length };
  if (state === null) return { opens: true, closes: false, next: marker };
  if (marker.char === state.char && marker.length >= state.length) {
    return { opens: false, closes: true, next: null };
  }
  return { opens: false, closes: false, next: state };
}

/** Whether the caret is on an opening delimiter line after the delimiter run.
 * The delimiter line is outside `caretInFence` by design, but Enter/paste there
 * must continue the source block instead of splitting the outline (OG's
 * `thing-at-point = source-block` behavior). */
export function caretOnOpeningFence(raw: string, offset: number): boolean {
  const target = Math.max(0, Math.min(offset, raw.length));
  let state: FenceState | null = null;
  let pos = 0;
  while (pos <= raw.length) {
    const nl = raw.indexOf("\n", pos);
    const end = nl === -1 ? raw.length : nl;
    const line = raw.slice(pos, end);
    const transition = transitionFence(state, line);
    if (target <= end) {
      const marker = FENCE_RUN.exec(line);
      return !!marker && transition.opens && target - pos >= marker[0].length;
    }
    state = transition.next;
    if (nl === -1) break;
    pos = end + 1;
  }
  return false;
}
