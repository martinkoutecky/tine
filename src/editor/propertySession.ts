// One edited Org buffer owns its parsed facts. Keep the visible projection and
// facets together until raw/format changes; never seed a fabricated render AST.
// O(block bytes), one visible-buffer parse when extending an accepted drawer.
// Other mutations, other surfaces and rendering after edit exit read raw normally.
import { blockRegions, type BlockIdentityFacts } from "../render/parse";
import { facetsOf, knownIdentityAbsent, type Facets } from "../render/facets";
import { joinProps, splitProps, type PropFormat } from "./properties";

export function propertyEditorSession() {
  let edited: { raw: string; format: PropFormat; hide: (key: string) => boolean; hiddenFacts: [string, string][]; parts: ReturnType<typeof splitProps>; facets: Facets; identity: BlockIdentityFacts } | undefined;
  return {
    identity(raw: string, format: PropFormat): BlockIdentityFacts {
      return edited?.raw === raw && edited.format === format ? edited.identity
        : { raw, format, value: knownIdentityAbsent(raw, format) ? null : blockRegions(raw, format).id?.value ?? null };
    },
    facets(raw: string, format: PropFormat): Facets {
      return edited?.raw === raw && edited.format === format ? edited.facets : facetsOf(raw, format);
    },
    split(raw: string, hide: (key: string) => boolean, format: PropFormat) {
      return edited?.raw === raw && edited.format === format && edited.hide === hide ? edited.parts : splitProps(raw, hide, format);
    },
    join(visible: string, hidden: string, previous: string, hide: (key: string) => boolean, format: PropFormat) {
      const raw = joinProps(visible, hidden, format);
      if (format === "org" && hidden && blockRegions(visible, format).properties.some((p) => p.primary)) {
        const previousHidden = edited?.raw === previous && edited.format === format && edited.hide === hide
          ? edited.hiddenFacts : blockRegions(previous, format).properties
            .filter((p) => p.primary && hide(p.key.toLowerCase())).map((p): [string, string] => [p.key, p.value]);
        const priorIdentity = this.identity(previous, format).value;
        const facts = facetsOf(visible, format);
        const own = blockRegions(visible, format).properties.filter((p) => p.primary);
        const ownCount = own.length;
        const hiddenFacts = own.filter((p) => hide(p.key.toLowerCase())).map((p): [string, string] => [p.key, p.value]);
        const parts = splitProps(visible, hide, format);
        edited = { raw, format, hide, hiddenFacts: [...hiddenFacts, ...previousHidden], identity: { raw, format, value: blockRegions(visible, format).id?.value ?? priorIdentity },
          parts: { visible: parts.visible, hidden: [parts.hidden, hidden].filter(Boolean).join("\n") },
          facets: { ...facts, properties: [...facts.properties.slice(0, ownCount), ...previousHidden, ...facts.properties.slice(ownCount)] } };
      } else edited = undefined;
      return raw;
    },
  };
}
