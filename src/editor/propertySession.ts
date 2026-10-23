// One edited buffer owns its parsed facts. Keep the visible projection and
// facets together until raw/format changes; never seed a fabricated render AST.
// O(block bytes), one visible-buffer parse when extending an accepted drawer.
// Other mutations, other surfaces and rendering after edit exit read raw normally.
import { blockRegions, type BlockIdentityFacts } from "../render/parse";
import { facetsOf, type Facets } from "../render/facets";
import { existingBlockId } from "../document";
import { hasLogbookDrawer, splitLogbook, hides, joinProps, splitProps, type HidePolicy, type PropFormat } from "./properties";

export function propertyEditorSession() {
  let edited: { raw: string; format: PropFormat; hide: HidePolicy; hiddenFacts: [string, string][]; parts: ReturnType<typeof splitProps>; facets: Facets; identity: BlockIdentityFacts } | undefined;
  return {
    begin() { edited = undefined; },
    identity(raw: string, format: PropFormat): BlockIdentityFacts {
      return edited?.raw === raw && edited.format === format ? edited.identity
        : { raw, format, value: existingBlockId(raw, format) };
    },
    facets(raw: string, format: PropFormat): Facets {
      return edited?.raw === raw && edited.format === format ? edited.facets : facetsOf(raw, format);
    },
    split(raw: string, hide: HidePolicy, format: PropFormat) {
      return edited?.raw === raw && edited.format === format && edited.hide === hide ? edited.parts : splitProps(raw, hide, format);
    },
    join(visible: string, hidden: string, previous: string, hide: HidePolicy, format: PropFormat) {
      // Like handwritten hidden properties, a supplied drawer belongs to this
      // active buffer until exit. Remove the previous history from the join and
      // cached transport so subsequent keystrokes cannot resurrect it. Keep the
      // literal typed layout, including text after :END:.
      const typedDrawer = hasLogbookDrawer(visible, format);
      if (typedDrawer) hidden = splitLogbook(hidden, "md").text;
      const raw = joinProps(visible, hidden, format, typedDrawer ? undefined : previous);
      const own = visible.includes(format === "org" ? ":" : "::")
        ? blockRegions(visible, format).properties.filter((p) => p.primary) : [];
      const previousHidden = () => {
        if (edited?.raw === previous && edited.format === format && edited.hide === hide) return edited.hiddenFacts;
        const prior = blockRegions(previous, format).properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
        return prior.filter(([key]) => hides(hide, key, prior));
      };
      // joinProps reattaches these accepted hidden facts to the parsed visible
      // primary properties. Supply that complete context without a second parse
      // of the rejoined Org buffer on every keystroke.
      const properties = own.length ? [...own.map((p): [string, string] => [p.key, p.value]), ...previousHidden()] : [];
      const typedHidden = own.some((p) => hides(hide, p.key, properties));
      if (typedHidden || typedDrawer) {
        // A property typed by hand must finish in this active buffer. Stripping
        // `quote:: ` as soon as the parser accepts it would put the remaining
        // keystrokes in the body. Reentry uses the ordinary split and hides it.
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
