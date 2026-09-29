/** Session-only section and group disclosure state for reference panels.
 * Reads and writes cost O(1) per page; reset on graph switch so identical page
 * names in different graphs cannot share a choice. */
export type ReferenceSection = "linked" | "unlinked";
const sections = new Map<string, boolean>();
const groups = new Map<string, Set<string>>();
const key = (section: ReferenceSection, page: string) => `${section}\0${page}`;

/** Return an explicit disclosure choice, or undefined for the section default. */
export function sectionOverride(section: ReferenceSection, page: string): boolean | undefined {
  return sections.get(key(section, page));
}

/** Store the user's explicit disclosure choice for this graph session. */
export function setSectionOverride(section: ReferenceSection, page: string, open: boolean): void {
  sections.set(key(section, page), open);
}

/** Return the current group collapse set; callers replace rather than mutate it. */
export function collapsedGroupsFor(section: ReferenceSection, page: string): Set<string> {
  return groups.get(key(section, page)) ?? new Set<string>();
}

/** Replace a page section's group collapse set. */
export function setCollapsedGroupsFor(section: ReferenceSection, page: string, value: Set<string>): void {
  groups.set(key(section, page), value);
}

/** Retire all reference disclosure choices after a graph switch. */
export function resetReferenceSectionState(): void {
  sections.clear();
  groups.clear();
}
