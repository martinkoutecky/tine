// One edited buffer owns its parsed facts. Keep the visible projection and
// facets together until raw/format changes; never seed a fabricated render AST.
// O(block bytes), one visible-buffer parse when extending an accepted drawer.
// Other mutations, other surfaces and rendering after edit exit read raw normally.
import { blockRegions, type BlockIdentityFacts } from "../render/parse";
import { facetsOf, type Facets } from "../render/facets";
import { existingBlockId } from "../document";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import { editorHidePolicy, hasLogbookDrawer, splitLogbook, hides, joinProps, rawOffsetToVisibleOffset, splitProps, type HidePolicy, type PropFormat } from "./properties";

const logbooks = (raw: string, format: PropFormat) => blockRegions(raw, format).drawers.filter(d => d.name.toLowerCase() === "logbook");
const ownHidden = (raw: string, format: PropFormat, hide: HidePolicy): [string, string][] => {
  const prior = blockRegions(raw, format).properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return prior.filter(([key]) => hides(hide, key, prior));
};

/** A drawer in the buffer replacing the block's hidden history: the rest of the
 *  buffer splices into `previous` as any text edit does, so every hidden row and
 *  the history keep their place, then the typed drawer takes the history's bytes.
 *  O(block bytes). */
function historyInPlace(visible: string, previous: string, hide: HidePolicy, format: PropFormat): string | null {
  const typed = splitLogbook(visible, format);
  const kept = joinProps(typed.text, splitProps(previous, hide, format).hidden, format, previous, hide);
  const drawers = logbooks(kept, format);
  if (!drawers.length) return null;
  const at = utf8ToUtf16Cursor(kept);
  let raw = kept;
  for (let i = drawers.length - 1; i >= 0; i--) {
    const start = at(drawers[i].range[0]), end = at(drawers[i].range[1]);
    const ending = kept.slice(start, end).match(/[\r\n]*$/)![0];
    raw = raw.slice(0, start) + (i === 0 ? typed.drawers + ending : "") + raw.slice(end);
  }
  return raw;
}

export function propertyEditorSession() {
  let edited: { raw: string; format: PropFormat; hide: HidePolicy; hiddenFacts: [string, string][]; parts: ReturnType<typeof splitProps>; facets: Facets; identity: BlockIdentityFacts } | undefined;
  return {
    begin() { edited = undefined; },
    /** Open this buffer with the block's LOGBOOK drawers in its text, each at its
     *  place ("Edit clock entries"): the supplied-drawer path of `join`, taken at
     *  mount instead of by typing. Re-entry after `begin` hides them again.
     *  Returns the caret, on the first CLOCK line, or null without a drawer.
     *  O(block bytes). */
    showLogbook(raw: string, hide: HidePolicy, format: PropFormat): number | null {
      hide = editorHidePolicy(hide);
      const drawers = logbooks(raw, format);
      if (!drawers.length) return null;
      const parts = splitProps(raw, hide, format);
      const at = utf8ToUtf16Cursor(raw);
      let visible = parts.visible, caret = 0;
      for (const drawer of [...drawers].reverse()) {
        const start = at(drawer.range[0]);
        const text = raw.slice(start, at(drawer.range[1])).replace(/[\r\n]+$/, "");
        // The drawer's raw start maps to where its hidden lines would appear.
        const place = rawOffsetToVisibleOffset(raw, start, hide, format);
        const lead = place > 0 ? "\n" : "", after = visible.slice(place);
        visible = visible.slice(0, place) + lead + text + (place === 0 && after ? "\n" : "") + after;
        caret = place + lead.length + (drawer.clocks.length ? at(drawer.clocks[0][0]) - start : 0);
      }
      edited = { raw, format, hide, hiddenFacts: ownHidden(raw, format, hide), parts: { visible, hidden: splitLogbook(parts.hidden, "md").text },
        identity: { raw, format, value: existingBlockId(raw, format) }, facets: facetsOf(raw, format) };
      return caret;
    },
    identity(raw: string, format: PropFormat): BlockIdentityFacts {
      return edited?.raw === raw && edited.format === format ? edited.identity
        : { raw, format, value: existingBlockId(raw, format) };
    },
    facets(raw: string, format: PropFormat): Facets {
      return edited?.raw === raw && edited.format === format ? edited.facets : facetsOf(raw, format);
    },
    split(raw: string, hide: HidePolicy, format: PropFormat) {
      hide = editorHidePolicy(hide);
      return edited?.raw === raw && edited.format === format && edited.hide === hide ? edited.parts : splitProps(raw, hide, format);
    },
    join(visible: string, hidden: string, previous: string, hide: HidePolicy, format: PropFormat) {
      hide = editorHidePolicy(hide);
      // Like handwritten hidden properties, a supplied drawer belongs to this
      // active buffer until exit. Remove the previous history from the join and
      // cached transport so subsequent keystrokes cannot resurrect it. Keep the
      // literal typed layout, including text after :END:, in the buffer; in the
      // file a drawer replacing history takes the history's place.
      const typedDrawer = hasLogbookDrawer(visible, format);
      if (typedDrawer) hidden = splitLogbook(hidden, "md").text;
      const raw = (typedDrawer && hasLogbookDrawer(previous, format) ? historyInPlace(visible, previous, hide, format) : null)
        ?? joinProps(visible, hidden, format, typedDrawer ? undefined : previous, hide);
      const own = visible.includes(format === "org" ? ":" : "::")
        ? blockRegions(visible, format).properties.filter((p) => p.primary) : [];
      const previousHidden = () => {
        if (edited?.raw === previous && edited.format === format && edited.hide === hide) return edited.hiddenFacts;
        return ownHidden(previous, format, hide);
      };
      // joinProps reattaches these accepted hidden facts to the parsed visible
      // primary properties. Supply that complete context without a second parse
      // of the rejoined Org buffer on every keystroke.
      const properties = own.length ? [...own.map((p): [string, string] => [p.key, p.value]), ...previousHidden()] : [];
      const typedHidden = own.some((p) => hides(hide, p.key, properties));
      const regrouped = format === "md" && hidden !== "" && own.length > 0
        && splitProps(raw, hide, format).visible !== visible;
      if (typedHidden || typedDrawer || regrouped) {
        // A property typed by hand must finish in this active buffer. Stripping
        // `tine.quote:: ` as soon as the parser accepts it would put the remaining
        // keystrokes in the body. Reentry uses the ordinary split and hides it.
        // Saving may also gather later Markdown properties into the first group
        // OG reads. Keep that saved layout out of the live text and caret, as OG
        // does in handler/editor.cljs::wrap-parse-block / editor-on-change!.
        const retained = previousHidden();
        const facts = facetsOf(visible, format);
        const ownCount = blockRegions(visible, format).properties.filter((p) => p.primary).length;
        edited = { raw, format, hide, hiddenFacts: retained, parts: { visible, hidden },
          identity: { raw, format, value: existingBlockId(raw, format) },
          facets: format === "org" ? { ...facts, properties: [...facts.properties.slice(0, ownCount), ...retained, ...facts.properties.slice(ownCount)] } : facetsOf(raw, format) };
      } else if (format === "org" && hidden && own.length) {
        const retained = previousHidden();
        const priorIdentity = this.identity(previous, format).value;
        const facts = facetsOf(visible, format);
        const ownCount = own.length;
        const hiddenFacts = own.filter((p) => hides(hide, p.key, properties)).map((p): [string, string] => [p.key, p.value]);
        const parts = splitProps(visible, hide, format);
        edited = { raw, format, hide, hiddenFacts: [...hiddenFacts, ...retained], identity: { raw, format, value: blockRegions(visible, format).id?.value ?? priorIdentity },
          parts: { visible: parts.visible, hidden: [parts.hidden, hidden].filter(Boolean).join("\n") },
          facets: { ...facts, properties: [...facts.properties.slice(0, ownCount), ...retained, ...facts.properties.slice(ownCount)] } };
      } else edited = undefined;
      return raw;
    },
  };
}
