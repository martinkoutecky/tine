// GH #417 (with #418's toolbar remainder): act on ONE occurrence of a block
// reference `((uuid))` or an embed `{{embed …}}` inside the block that contains
// it (its host). The occurrence is located by the parser's source span of the
// rendered token, so two identical references in one block stay distinct.
//
// "Edit link" opens the host's editor with the occurrence selected (OG's
// double-click on an embed edits the host block, block.cljs `edit-parent-block`);
// "Remove" deletes the occurrence's text from the host only (OG
// `delete-current-ref!`), never the referenced block or page, as one undo step
// through the ordinary block save path. "Replace with text" and "Replace with embed"
// (OG `replace-ref-with-text!` / `replace-ref-with-embed!`) rewrite the occurrence
// the same way.
import { createContext, createMemo, useContext, type Accessor } from "solid-js";
import { msg } from "./i18n";
import { blockWritable, formatForBlock, formatForPage, setRaw, withUndoUnit, node as docNode } from "./document";
import { backend } from "./backend";
import { graphOwner, readOwned } from "./owned";
import { graphMeta } from "./graphSession";
import { startEditing } from "./editorController";
import { occurrenceSelection } from "./components/ReferenceEvidence";
import { pushToast } from "./toasts";
import { rebulletedSourceByteToRawByte, utf8ByteToUtf16Offset } from "./render/spans";
import { utf8ToUtf16Cursor } from "./render/utf16Cursor";
import { blockRegions, parserReady } from "./render/parse";
import { isRenderHiddenProp } from "./render/block";
import { hideKeys, splitProps } from "./editor/properties";
import type { Format, Span } from "./render/ast";

export interface RefOccurrence {
  /** The block whose text contains the occurrence. */
  host: string;
  /** The parser's source span of the rendered token (UTF-8 bytes, rebulleted source). */
  span: Span;
  /** "ref" for `((uuid))` / `[label](((uuid)))`, "embed" for `{{embed …}}`. */
  kind: "ref" | "embed";
  /** Text the occurrence must contain: the uuid of a reference, the target of an embed. */
  target: string;
  /** The host's whole text when the occurrence was taken from the rendered view;
   *  the span addresses this text and no other. */
  raw: string;
}

/** The occurrence as it is SHOWN: its rendered span with the host text that
 *  span addresses, taken when the view renders and re-taken (a new object) on
 *  every change of that text. Actions bind the object they were rendered with
 *  (a keyed `<Show>`), so a host change replaces the buttons, dropping any focus
 *  or half-done press on them, and an action that still holds an older object
 *  (an open menu) refuses in `rawRange`. Never take the snapshot at click time:
 *  the text read then may already differ from what the user was shown. */
export function createShownOccurrence(
  at: () => { host: string | undefined; span: Span | undefined; kind: RefOccurrence["kind"]; target: string } | undefined,
): Accessor<RefOccurrence | undefined> {
  return createMemo(() => {
    const a = at();
    const raw = a?.host && a.span ? docNode(a.host)?.raw : undefined;
    return raw === undefined ? undefined : { host: a!.host!, span: a!.span!, kind: a!.kind, target: a!.target, raw };
  });
}

/** An occurrence with the rendered host instance (see `useOccurrenceOwner`) that
 *  Edit link opens; the block-reference context menu carries it. */
export type OwnedOccurrence = RefOccurrence & { owner: string | null };

/** The rendered block instance an occurrence's actions belong to. Rendered.tsx
 *  provides it, so Edit link turns exactly that copy of the host into an editor. */
export const OccurrenceHostContext = createContext<{ id: string; owner: string } | null>(null);

/** The rendered instance owning `host`, for `editOccurrence`; null when the
 *  nearest rendered block is another one (the editor then follows the caret's surface). */
export function useOccurrenceOwner(host: () => string | undefined): () => string | null {
  const ctx = useContext(OccurrenceHostContext);
  return () => (ctx && ctx.id === host() ? ctx.owner : null);
}

/** The occurrence's UTF-16 range in the host's raw, or null when the host's
 *  text is no longer exactly the text the occurrence was taken from (an
 *  external editor, a sync delivery, a reload or another surface changed it
 *  while a menu or preview stayed open): a span into changed text can address a
 *  different reference or a code literal, so any change refuses. */
function rawRange(o: RefOccurrence): { start: number; end: number } | null {
  const raw = docNode(o.host)?.raw;
  if (raw === undefined || raw !== o.raw) return null;
  const start = utf8ByteToUtf16Offset(raw, rebulletedSourceByteToRawByte(raw, o.span[0]));
  const end = utf8ByteToUtf16Offset(raw, rebulletedSourceByteToRawByte(raw, o.span[1]));
  const text = raw.slice(start, end);
  const shaped = o.kind === "embed" ? text.startsWith("{{") && text.endsWith("}}") : text.includes("((");
  return shaped && text.toLowerCase().includes(o.target.toLowerCase()) ? { start, end } : null;
}

/** Whether Edit link and Remove may act: the host exists and its page accepts edits. */
export function occurrenceEditable(o: RefOccurrence | undefined): o is RefOccurrence {
  return !!o && !!docNode(o.host) && blockWritable(o.host);
}

function reportStale(): false {
  pushToast(msg("ref-occurrence-changed"), "warn");
  return false;
}

/** Open the host's editor with the occurrence's text selected. `owner` is the
 *  rendered host instance, so only that copy of the block turns into an editor. */
export function editOccurrence(o: RefOccurrence, owner: string | null): boolean {
  const n = docNode(o.host);
  const range = rawRange(o);
  if (!n || !range) return reportStale();
  startEditing(o.host, occurrenceSelection(n.raw, range, n.page), owner);
  return true;
}

/** Remove the occurrence's text from its host, with one adjacent space or tab
 *  (OG removes one whitespace before the reference; a line break is kept so
 *  lines never merge). The block itself stays, even when it ends up empty. */
export function removeOccurrence(o: RefOccurrence): boolean {
  const n = docNode(o.host);
  const range = rawRange(o);
  if (!n || !range || !blockWritable(o.host)) return reportStale();
  const raw = n.raw;
  const blank = (ch: string | undefined) => ch === " " || ch === "\t";
  let { start, end } = range;
  if (blank(raw[start - 1])) start -= 1;
  else if (blank(raw[end])) end += 1;
  const next = raw.slice(0, start) + raw.slice(end);
  withUndoUnit(o.kind === "embed" ? "remove-embed" : "remove-reference", [n.page], () => {
    setRaw(o.host, next, { timetracking: false });
  });
  return true;
}

/** The referenced block's text as OG's "Replace with text" inserts it
 *  (`replace-ref-with-text!`): its raw with the hidden built-in properties and
 *  the LOGBOOK drawer removed, every other line kept as written (visible
 *  properties, nested markup, a leading task marker). Which lines are
 *  properties and drawers is the parser's answer, so a `id:: x` line inside a
 *  code fence stays text (OG's line pattern removes the first such line
 *  anywhere); which keys are hidden is the one render predicate
 *  (`isRenderHiddenProp`), a superset of OG's list that also drops `title::`,
 *  `filters::` and `tine.*` view keys. Trailing line breaks are dropped so the
 *  text sits where the reference stood. */
export function referenceReplacementText(raw: string, format: Format, userHidden: readonly string[] = []): string {
  if (!parserReady()) return raw;
  const regions = blockRegions(raw, format);
  if (regions.quarantined) return raw;
  let text = raw;
  const at = utf8ToUtf16Cursor(raw);
  const logbooks = regions.drawers
    .filter((d) => d.name.toLowerCase() === "logbook")
    .map((d): [number, number] => [at(d.range[0]), at(d.range[1])])
    .sort((x, y) => y[0] - x[0]);
  for (const [a, b] of logbooks) text = text.slice(0, a) + text.slice(b);
  const keys = regions.properties.filter((p) => p.primary && isRenderHiddenProp(p.key, userHidden)).map((p) => p.key);
  if (keys.length) text = splitProps(text, hideKeys(keys, true), format === "org" ? "org" : "md").visible;
  return text.replace(/\n+$/, "");
}

/** Replace the occurrence's whole token with `replacement` in its host, one
 *  undo step through the ordinary block save. A labelled reference
 *  `[label](((uuid)))` is replaced whole, label included, as Remove does (OG
 *  replaces only the inner `((uuid))` and leaves `[label](…)` around it, which is
 *  no longer a link). Refuses, changing nothing, when the host's text is no
 *  longer the text the occurrence was shown with. */
function replaceOccurrence(o: RefOccurrence, replacement: string, unit: string): boolean {
  const n = docNode(o.host);
  const range = rawRange(o);
  if (!n || !range || !blockWritable(o.host)) return reportStale();
  const next = n.raw.slice(0, range.start) + replacement + n.raw.slice(range.end);
  withUndoUnit(unit, [n.page], () => {
    setRaw(o.host, next, { timetracking: false });
  });
  return true;
}

/** OG "Replace with text": the reference becomes the referenced block's text. */
export function replaceOccurrenceWithText(o: RefOccurrence, target: { raw: string; format: Format }, userHidden: readonly string[] = []): boolean {
  return replaceOccurrence(o, referenceReplacementText(target.raw, target.format, userHidden), "replace-reference-with-text");
}

/** OG "Replace with embed": the reference becomes `{{embed ((uuid))}}`. */
export function replaceOccurrenceWithEmbed(o: RefOccurrence, uuid: string): boolean {
  return replaceOccurrence(o, `{{embed ((${uuid}))}}`, "replace-reference-with-embed");
}

/** The menu's "Replace with text": read the referenced block (the loaded node,
 *  else the backend's copy for a block on a page that is not open), then replace.
 *  A reference whose target cannot be read changes nothing and says so. */
export async function replaceOccurrenceWithTargetText(o: RefOccurrence, uuid: string): Promise<boolean> {
  const live = docNode(uuid);
  let target: { raw: string; format: Format } | null = live ? { raw: live.raw, format: formatForBlock(uuid) } : null;
  if (!target) {
    try {
      const result = await readOwned(graphOwner(), backend().resolveBlock(uuid));
      const group = result.kind === "current" ? result.value : null;
      const block = group?.blocks[0];
      if (group && block) target = { raw: block.raw, format: formatForPage(group.page) };
    } catch { /* reported below */ }
  }
  if (!target) {
    pushToast(msg("ref-occurrence-target-unreadable"), "warn");
    return false;
  }
  return replaceOccurrenceWithText(o, target, graphMeta()?.block_hidden_properties ?? []);
}
