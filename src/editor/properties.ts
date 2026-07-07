// Pure helpers for reading/editing `key:: value` property lines — a block's
// continuation lines or a page's pre-block. No store/DOM, so unit-testable.

export const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

// Built-in properties hidden from the editor by default (like OG): `id::`,
// `collapsed::`, and `logseq.order-list-type::` (the numbered-list marker) are
// kept in the file for persistence but never shown in the edit textarea.
// Annotation (PDF highlight) blocks instead hide ALL properties and edit only
// their text.
const BUILTIN_HIDDEN = new Set(["id", "collapsed", "logseq.order-list-type"]);
/** Hide just the built-in `id::`/`collapsed::` properties (normal blocks). */
export const isBuiltinHidden = (key: string): boolean => BUILTIN_HIDDEN.has(key);
/** Hide metadata that should not surface while editing through a sheet cell. */
export const isSheetCellHidden = (key: string): boolean =>
  isBuiltinHidden(key) || key.toLowerCase().startsWith("tine.");
/** Hide every property (annotation blocks edit only their text). */
export const hideAll = (_key: string): boolean => true;

const FENCE_RE = /^\s*(`{3,}|~{3,})/;

function fenceMarker(line: string): string | null {
  const m = FENCE_RE.exec(line);
  return m ? m[1][0] : null;
}

function fenceTransition(
  fence: string | null,
  line: string
): { opens: boolean; closes: boolean; next: string | null } {
  const ch = fenceMarker(line);
  if (ch === null) return { opens: false, closes: false, next: fence };
  if (fence === null) return { opens: true, closes: false, next: ch };
  if (ch === fence) return { opens: false, closes: true, next: null };
  return { opens: false, closes: false, next: fence };
}

function propLineKey(line: string): string | null {
  const m = /^\s*([A-Za-z0-9_./-]+)::/.exec(line);
  return m ? m[1].toLowerCase() : null;
}

/** Whether a textarea caret offset is inside a fenced code region. The fence
 *  delimiter lines themselves are outside; the content lines between them are
 *  inside, including an unterminated fence while the user is editing. */
export function caretInFence(raw: string, offset: number): boolean {
  const target = Math.max(0, Math.min(offset, raw.length));
  let fence: string | null = null;
  let pos = 0;
  while (pos <= raw.length) {
    const nl = raw.indexOf("\n", pos);
    const end = nl === -1 ? raw.length : nl;
    const line = raw.slice(pos, end);
    const t = fenceTransition(fence, line);
    if (target <= end) return fence !== null && !t.closes;
    fence = t.next;
    if (nl === -1) break;
    pos = end + 1;
  }
  return fence !== null;
}

/** Split a block's raw into the editor-visible text and the hidden property
 *  lines. Fence-aware: a `key:: value` line inside a ```/~~~ code fence stays
 *  visible content — it must NOT be pulled out as metadata and reattached
 *  outside the fence (which would corrupt the code on focus+blur). `isHidden`
 *  selects which property keys are hidden (e.g. {@link isBuiltinHidden} or
 *  {@link hideAll}). Inverse of {@link joinProps}. */
export function splitProps(
  raw: string,
  isHidden: (key: string) => boolean
): { visible: string; hidden: string } {
  const { visible, hidden } = splitPropsInternal(raw, isHidden);
  return { visible, hidden };
}

function splitPropsInternal(
  raw: string,
  isHidden: (key: string) => boolean,
  rawOffset?: number
): { visible: string; hidden: string; visibleOffset?: number } {
  const vis: string[] = [];
  const hid: string[] = [];
  let fence: string | null = null;
  const target = rawOffset == null ? null : Math.max(0, Math.min(rawOffset, raw.length));
  let visibleLen = 0;
  let visibleOffset: number | null = null;
  let rawPos = 0;
  for (const l of raw.split("\n")) {
    const rawStart = rawPos;
    const rawEnd = rawStart + l.length;
    const t = fenceTransition(fence, l);
    let visible = false;
    if (t.opens || t.closes) {
      fence = t.next;
      visible = true;
    } else if (fence !== null) {
      visible = true; // inside a code fence — never metadata
    } else {
      const k = propLineKey(l);
      visible = !(k && isHidden(k));
    }
    if (visible) {
      const lineVisibleStart = visibleLen + (vis.length > 0 ? 1 : 0);
      const lineVisibleEnd = lineVisibleStart + l.length;
      if (target != null && visibleOffset == null && target >= rawStart && target <= rawEnd) {
        visibleOffset = lineVisibleStart + (target - rawStart);
      }
      vis.push(l);
      visibleLen = lineVisibleEnd;
    } else {
      if (target != null && visibleOffset == null && target >= rawStart && target <= rawEnd) {
        visibleOffset = visibleLen;
      }
      hid.push(l);
    }
    rawPos = rawEnd + 1;
  }
  return {
    visible: vis.join("\n"),
    hidden: hid.join("\n"),
    visibleOffset: target == null ? undefined : (visibleOffset ?? visibleLen),
  };
}

/** Map a UTF-16 offset in raw block text into the textarea's visible buffer,
 *  using the same fence-aware hidden-property split as {@link splitProps}. When
 *  the raw offset falls inside a hidden property line, it maps to the edit point
 *  where that removed line would have appeared. */
export function rawOffsetToVisibleOffset(
  raw: string,
  rawOffset: number,
  isHidden: (key: string) => boolean
): number {
  return splitPropsInternal(raw, isHidden, rawOffset).visibleOffset ?? 0;
}

/** Reattach hidden property lines below the visible text. A metadata-only block
 *  (empty visible) is just its hidden lines — no spurious leading newline. */
export function joinProps(visible: string, hidden: string): string {
  return hidden ? (visible ? `${visible}\n${hidden}` : hidden) : visible;
}

/** First value for `key` (case-insensitive) in a property block, or null. */
export function readPropertyValue(block: string | null, key: string): string | null {
  if (!block) return null;
  for (const l of block.split("\n")) {
    const m = PROP_LINE.exec(l);
    if (m && m[1].toLowerCase() === key.toLowerCase()) return m[2].trim();
  }
  return null;
}

/** Add / replace / remove a `key:: value` line. A null or empty value removes
 *  the key. Other property lines are preserved (blank lines dropped); returns
 *  null when nothing is left (so an emptied pre-block isn't written as "").  */
export function upsertPropertyLine(
  block: string | null,
  key: string,
  value: string | null
): string | null {
  const kept = (block ?? "")
    .split("\n")
    .filter((l) => {
      const m = PROP_LINE.exec(l);
      return !(m && m[1].toLowerCase() === key.toLowerCase());
    })
    .filter((l) => l.trim() !== "");
  const v = value == null ? null : value.trim();
  if (v) kept.push(`${key}:: ${v}`);
  return kept.length ? kept.join("\n") : null;
}

/** The page-level properties we surface in the page-properties panel, with a
 *  one-line description and whether the value is a boolean toggle. */
export interface PagePropSpec {
  key: string;
  label: string;
  hint: string;
  kind: "text" | "bool" | "list";
}
export const PAGE_PROP_SPECS: PagePropSpec[] = [
  { key: "alias", label: "Aliases", hint: "Other names this page answers to in [[links]] (comma-separated)", kind: "list" },
  { key: "tags", label: "Tags", hint: "Page-level tags (comma-separated)", kind: "list" },
  { key: "title", label: "Display title", hint: "Override the shown title (the file name stays the same)", kind: "text" },
  { key: "icon", label: "Icon", hint: "An emoji/character shown with the title", kind: "text" },
  { key: "public", label: "Public", hint: "Include this page when exporting/publishing public pages", kind: "bool" },
];
