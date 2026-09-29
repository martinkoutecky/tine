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

const MATH_DELIM = "$$";

/** Advance display-math state across text known not to be inside a code fence.
 * Every `$$` toggles, so `$$x$$` on one line opens and closes again (net
 * outside) while a lone `$$` opens a multi-line environment. */
function toggleDisplayMath(open: boolean, text: string): boolean {
  let at = text.indexOf(MATH_DELIM);
  while (at !== -1) {
    open = !open;
    at = text.indexOf(MATH_DELIM, at + MATH_DELIM.length);
  }
  return open;
}

/** Whether a caret offset sits inside an open `$$ … $$` display-math environment.
 *
 * This is a DELIBERATE DIVERGENCE FROM OG, not a parity fix (GH #278, Martin's
 * call). OG's Enter dwim recognises only ``` and `#+BEGIN_`
 * (`frontend/util/thingatpt.cljs` `admonition&src-at-point`), so pressing Enter
 * inside `$$ … $$` splits the bullet and breaks the environment there too. A
 * multi-line display-math block is one piece of content, so Tine keeps Enter
 * inside it the way it does for a code fence.
 *
 * Confined to the editor's Enter decision on purpose: the parser, the bytes on
 * disk, and property classification (`classifyLines`) are untouched, so nothing
 * about how a block is stored or read changes. `$$` inside a code fence is
 * literal text and opens nothing. */
export function caretInDisplayMath(raw: string, offset: number): boolean {
  const target = Math.max(0, Math.min(offset, raw.length));
  let fence: FenceState | null = null;
  let open = false;
  let pos = 0;
  while (pos <= raw.length) {
    const nl = raw.indexOf("\n", pos);
    const end = nl === -1 ? raw.length : nl;
    const line = raw.slice(pos, end);
    const transition = transitionFence(fence, line);
    const insideFence = fence !== null || transition.opens;
    if (target <= end) {
      // Only what precedes the caret on its own line can change the state.
      return insideFence ? false : toggleDisplayMath(open, line.slice(0, target - pos));
    }
    if (!insideFence) open = toggleDisplayMath(open, line);
    fence = transition.next;
    if (nl === -1) break;
    pos = end + 1;
  }
  return open;
}

/** Display-math state after consuming `text` whole — for the double-Enter exit,
 * which asks "was the environment open before this blank line?". */
export function displayMathOpenAfter(text: string): boolean {
  let fence: FenceState | null = null;
  let open = false;
  for (const line of text.split("\n")) {
    const transition = transitionFence(fence, line);
    if (fence === null && !transition.opens) open = toggleDisplayMath(open, line);
    fence = transition.next;
  }
  return open;
}

/** Whether a line closes an open display-math environment. */
export function closesDisplayMath(line: string): boolean {
  return line.includes(MATH_DELIM);
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
