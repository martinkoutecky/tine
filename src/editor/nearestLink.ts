// "The link at the caret", for GH #274 (Ctrl+O to open a page without the mouse).
//
// Transcribed from OG rather than invented, because this is a keyboard contract
// users already have muscle memory for:
// `frontend/util/thingatpt.cljs` `extract-nearest-link-from-text`, driven by
// `frontend/handler/editor.cljs` `get-nearest-page-or-url` / `get-nearest-page`
// and bound at `frontend/modules/shortcut/config.cljs:214` (`:editor/follow-link`,
// `mod+o`) and `:217` (`:editor/open-link-in-sidebar`, `mod+shift+o`).
//
// Note what OG's rule actually is, because it is not "the link under the caret":
// it collects EVERY candidate in the block and picks the one NEAREST the caret,
// so a caret anywhere in a block with a single link still follows it. That is
// friendlier than it sounds and it is the behaviour people are used to.

export type NearestLinkKind = "page" | "tag" | "block" | "url";

export interface NearestLink {
  kind: NearestLinkKind;
  /** The dereferenced target: page name, tag name, block uuid, or the URL. */
  value: string;
  /** Offsets of the whole match in the source text. */
  start: number;
  end: number;
}

// OG's exact patterns. `url-regex` is deliberately not a general URL matcher —
// OG's own comment says link/plain-link "incorrectly detects words as urls".
const PAGE_REF = /\[\[([^\]]+)\]\]/g;
const BLOCK_REF = /\(\(([^)]+)\)\)/g;
const TAG = /#\S+/g;
const URL = /[^\s([]+:\/\/[^\s)\]]+/g;

interface Candidate {
  kind: NearestLinkKind;
  text: string;
  start: number;
}

function collect(text: string, pattern: RegExp, kind: NearestLinkKind): Candidate[] {
  const out: Candidate[] = [];
  for (const match of text.matchAll(pattern)) {
    if (match.index === undefined) continue;
    out.push({ kind, text: match[0], start: match.index });
  }
  return out;
}

/** The link nearest `caret`, or null. `includeUrls` mirrors OG's split: the
 *  follow-link command passes the URL pattern as an "additional pattern", the
 *  open-in-sidebar command does not (a URL has no sidebar representation). */
export function nearestLink(
  text: string,
  caret: number,
  options: { includeUrls?: boolean } = {}
): NearestLink | null {
  const candidates = [
    ...collect(text, PAGE_REF, "page"),
    ...collect(text, BLOCK_REF, "block"),
    ...collect(text, TAG, "tag"),
    ...(options.includeUrls ? collect(text, URL, "url") : []),
  ];
  if (candidates.length === 0) return null;

  // OG's ranking: 0 when the caret is inside the match, otherwise a NEGATIVE
  // distance, sorted descending — so "inside" wins and, failing that, the
  // closest match does. A stable sort keeps the earlier pattern on a tie, which
  // is what OG's concat order gives.
  const score = (candidate: Candidate) => {
    const end = candidate.start + candidate.text.length;
    if (caret < candidate.start) return caret - candidate.start;
    if (caret > end) return end - caret;
    return 0;
  };
  const best = candidates
    .map((candidate, index) => ({ candidate, index, score: score(candidate) }))
    .sort((a, b) => b.score - a.score || a.index - b.index)[0].candidate;

  const end = best.start + best.text.length;
  const value = best.kind === "url"
    ? best.text
    : best.kind === "tag"
      ? best.text.slice(1)
      : best.text.slice(2, -2);
  const trimmed = value.trim();
  if (trimmed === "") return null;
  return { kind: best.kind, value: trimmed, start: best.start, end };
}
