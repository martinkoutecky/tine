import { createSignal, type Accessor } from "solid-js";
import { collapsedGroupsFor, setCollapsedGroupsFor, type ReferenceSection } from "./referenceSectionState";
import { pageIdentityKey } from "./pageIdentity";
import type { RefGroup } from "./types";

/** Session collapse state for one reference panel. O(collapsed groups) per
 * toggle, O(shown groups) for bulk; no fetching or additional effect loop. */
export function createReferenceGroupCollapse(section: ReferenceSection, page: Accessor<string>) {
  const [collapsed, setCollapsed] = createSignal(collapsedGroupsFor(section, page()));
  const replace = (value: Set<string>) => {
    setCollapsedGroupsFor(section, page(), value);
    setCollapsed(value);
  };
  return {
    reload: () => setCollapsed(collapsedGroupsFor(section, page())),
    groupCollapsed: (group: RefGroup) => collapsed().has(pageIdentityKey(group.page)),
    setGroupCollapsed: (group: RefGroup, value: boolean) => {
      const next = new Set(collapsed());
      if (value) next.add(pageIdentityKey(group.page));
      else next.delete(pageIdentityKey(group.page));
      replace(next);
    },
    setAll: (shown: readonly RefGroup[], value: boolean) => replace(value ? new Set(shown.map((group) => pageIdentityKey(group.page))) : new Set()),
  };
}
