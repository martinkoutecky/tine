// The mock backend's answers over `alias::` declarations: a name's alias
// owners and a page's alias group (D-20261019-AL1/AL2), as the real backend's
// `Resolved::Alias` and `InventoryScope::AliasGroup`.
import type { AliasGroup, PageDto } from "./types";

/** A page's `alias::` names (pre-block only, as the mock reads them). */
function declaredAliases(p: PageDto): string[] {
  return (p.pre_block ?? "").split(/\n/)
    .filter((line) => /^alias::\s*/i.test(line))
    .flatMap((line) => line.replace(/^alias::\s*/i, "").split(",").map((alias) => alias.trim()).filter(Boolean));
}

export function mockAliases(all: PageDto[], identity: (name: string) => string, pagePath: (p: PageDto) => string) {
  /** The paths of the pages declaring `name`, in plain text order (the owner rule). */
  const aliasOwners = (name: string): string[] =>
    all.filter((p) => declaredAliases(p).some((alias) => identity(alias) === identity(name))).map(pagePath).sort();
  /** The backend's transitive alias component, over the mock's declarations. */
  async function pageAliasGroup(name: string): Promise<AliasGroup> {
    const next = new Map<string, Set<string>>();
    const spelling = new Map<string, string>();
    const link = (a: string, b: string) => {
      if (!next.has(a)) next.set(a, new Set());
      next.get(a)!.add(b);
    };
    for (const page of all) {
      for (const alias of declaredAliases(page)) {
        link(identity(page.name), identity(alias));
        link(identity(alias), identity(page.name));
        if (!spelling.has(identity(alias))) spelling.set(identity(alias), alias);
      }
    }
    const own = identity(name);
    const group = new Set<string>();
    for (const pending = [own]; pending.length; ) {
      const key = pending.pop()!;
      if (group.has(key)) continue;
      group.add(key);
      pending.push(...(next.get(key) ?? []));
    }
    group.delete(own);
    const aliases: string[] = [];
    const pages: string[] = [];
    for (const key of group) {
      const page = all.find((p) => identity(p.name) === key);
      if (page) pages.push(page.name);
      else aliases.push(spelling.get(key) ?? key);
    }
    const byName = (a: string, b: string) => a.toLowerCase().localeCompare(b.toLowerCase());
    return { aliases: aliases.sort(byName), pages: pages.sort(byName) };
  }
  return { aliasOwners, pageAliasGroup };
}
