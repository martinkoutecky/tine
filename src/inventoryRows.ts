// The pure readers of `page_inventory` rows: which file a resolved name opens
// (`ownerPath`) and what `[[name]]` opens (`linkTargetIn`). They hold no state
// and cache nothing: `pageIndex.ts` applies them to the app's one held
// inventory, and the published site applies the same functions to the rows a
// snapshot carries (`publishedAnswers.ts`; CM step 2, census Q13), so a
// published link opens what it opens in the app by one piece of code (I-12).
// Kept free of the backend so the published backend can import it without a
// module cycle.
import { pageIdentityKey } from "./pageIdentity";
import { absentLinkIsJournal, journalTitle, parseJournalTitle } from "./journal";
import type { PageInventoryEntry, PageKind, ResolvedPage } from "./types";

/** Page-kind and journal-kind rows by `key` (the last row of a key wins, as
 *  the backend's list order leaves physical spelling twins equivalent). */
export interface InventoryRowIndex {
  readonly pages: Map<string, PageInventoryEntry>;
  readonly journals: Map<string, PageInventoryEntry>;
}

/** Index rows by kind and key. O(rows). */
export function indexInventoryRows(entries: readonly PageInventoryEntry[]): InventoryRowIndex {
  const pages = new Map<string, PageInventoryEntry>();
  const journals = new Map<string, PageInventoryEntry>();
  for (const entry of entries) (entry.is_journal ? journals : pages).set(entry.key, entry);
  return { pages, journals };
}

/** The file a resolved name opens: the claimant order's owner for
 *  `existing` (its `id`), the page an alias-only name opens for `alias` (the
 *  backend lists owners each once in relative-path order and the first is the
 *  owner, D-20261019-AL2/CI4), nothing for `absent`. The backend decides; this
 *  is the frontend's one reader of that answer (CM step 2, I-12; a guard test
 *  forbids reading `owners[0]` anywhere else). */
export function ownerPath(
  target: ResolvedPage | { readonly kind: "alias"; readonly owners: readonly string[] } | null | undefined,
): string | undefined {
  if (target?.kind === "existing") return target.id;
  if (target?.kind === "alias") return target.owners[0];
  return undefined;
}

/** Where a link or a named route goes: a page name and its kind. */
export interface LinkDestination { readonly name: string; readonly kind: PageKind }

/** Configured day lookup over the held typed rows; literal claims are checked first.
 *  `readKey` tracks the canonical bucket, including a currently missing row. */
export function journalRowIn(index: InventoryRowIndex | null | undefined, name: string, readKey?: (key: string) => void): PageInventoryEntry | undefined {
  if (!/\d/.test(name)) return undefined;
  const day = parseJournalTitle(name);
  if (!day) return undefined;
  const key = pageIdentityKey(journalTitle(day));
  readKey?.(key);
  return index?.journals.get(key);
}

/** What `[[name]]` opens over `index`. The store's answer
 *  (`WholeGraph::link_target`, carried as the row's `link`) for every name the
 *  rows list; a name they do not list is claimed by no file, alias or
 *  reference, so its answer is an absent page whose kind the Rust rule
 *  `JournalFormat::absent_link_is_journal` gives (through wasm). Decides
 *  nothing itself. O(1). */
export function linkTargetIn(index: InventoryRowIndex | null | undefined, name: string, configuredDates = false, readKey?: (key: string) => void): LinkDestination {
  const key = pageIdentityKey(name);
  const page = index?.pages.get(key);
  if (page) return page.link ?? { name: page.target.kind === "absent" ? name : page.name, kind: "page" };
  const journal = index?.journals.get(key);
  if (journal) return (configuredDates ? journal.link : undefined) ?? { name: journal.name, kind: "journal" };
  if (configuredDates && /\d/.test(name)) {
    const canonical = journalRowIn(index, name, readKey);
    if (canonical) return canonical.link ?? { name: canonical.name, kind: "journal" };
  }
  return { name, kind: absentLinkIsJournal(name) ? "journal" : "page" };
}
