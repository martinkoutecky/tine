import { batch, type Accessor } from "solid-js";
import { ChangeKeyRegistry } from "./changeKeyRegistry";
import { collapsedGroupsFor, setCollapsedGroupsFor, type ReferenceSection } from "./referenceSectionState";
import { pageIdentityKey } from "./pageIdentity";
import type { RefGroup } from "./types";

/** Local panel snapshot with keyed notifications. Whole replacements and
 * explicit reloads visit their choices; a single toggle touches one key. */
export function createKeyedGroupCollapse<G>(keyOf: (group: G) => string,
  initial: () => Set<string> = () => new Set(),
  persist?: (value: Set<string>) => void) {
  let collapsed = initial();
  const changes = new ChangeKeyRegistry();
  const replace = (value: Set<string>) => {
    const previous = collapsed;
    collapsed = value;
    batch(() => {
      for (const key of previous) if (!value.has(key)) changes.advance(key);
      for (const key of value) if (!previous.has(key)) changes.advance(key);
    });
  };
  return {
    reload: () => replace(initial()),
    groupCollapsed: (group: G) => { const key = keyOf(group); changes.read(key); return collapsed.has(key); },
    setGroupCollapsed: (group: G, value: boolean) => {
      const key = keyOf(group);
      const previous = collapsed.has(key);
      if (value) collapsed.add(key); else collapsed.delete(key);
      persist?.(collapsed);
      if (previous !== value) changes.advance(key);
    },
    setAll: (shown: readonly G[], value: boolean) => {
      const next = value ? new Set(shown.map(keyOf)) : new Set<string>();
      persist?.(next);
      replace(next);
    },
  };
}

/** Session choices remain separate snapshots for each mounted panel. */
export function createReferenceGroupCollapse(section: ReferenceSection, page: Accessor<string>) {
  return createKeyedGroupCollapse<RefGroup>(group => pageIdentityKey(group.page),
    () => collapsedGroupsFor(section, page()),
    value => setCollapsedGroupsFor(section, page(), value));
}
