// The pure readers of `page_inventory` rows: which file a resolved name opens
// (`ownerPath`) and what `[[name]]` opens (`linkTargetIn`). They hold no state
// and cache nothing: `pageIndex.ts` applies them to the app's one held
// inventory, and the published site applies the same functions to the rows a
// snapshot carries (`publishedAnswers.ts`; CM step 2, census Q13), so a
// published link opens what it opens in the app by one piece of code (I-12).
// Kept free of the backend so the published backend can import it without a
// module cycle.
import { pageIdentityKey } from "./pageIdentity";
import { absentLinkIsJournal, journalOpeningTitle } from "./journal";
import type { PageInventoryEntry, PageKind, ResolvedPage } from "./types";

/** Page-kind and journal-kind rows by `key` (the last row of a key wins, as
 *  the backend's list order leaves physical spelling twins equivalent). */
export interface InventoryRowIndex {
  readonly pages: Map<string, PageInventoryEntry>;
  readonly journals: Map<string, PageInventoryEntry>;
  linkRow(key: string): PageInventoryEntry | undefined;
}

/** Index a frozen v1 publication's rows. Its lookup rows preserve the
 *  published page slot before the journal slot. Live inventory supplies
 *  a single native opening on every row of the key. O(rows). */
export function indexInventoryRows(entries: readonly PageInventoryEntry[]): InventoryRowIndex {
  const pages = new Map<string, PageInventoryEntry>();
  const journals = new Map<string, PageInventoryEntry>();
  for (const entry of entries) (entry.is_journal ? journals : pages).set(entry.key, entry);
  return { pages, journals, linkRow: (key) => pages.get(key) ?? journals.get(key) };
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

/** The journal row named by the Rust date answer.
 *  `readKey` tracks the canonical bucket, including a currently missing row. */
export function journalRowIn(index: InventoryRowIndex | null | undefined, name: string, readKey?: (key: string) => void): PageInventoryEntry | undefined {
  const title = journalOpeningTitle(name);
  if (title === undefined) return undefined;
  const key = pageIdentityKey(title);
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
  const row = index?.linkRow(key);
  if (row?.link) {
    // An ordinary name without a claimant keeps the requested spelling.
    if (configuredDates && row.target.kind === "absent" && row.link.kind === "page" && row.link.name === row.name) {
      return { name, kind: row.link.kind };
    }
    return row.link;
  }
  if (configuredDates) {
    const title = journalOpeningTitle(name);
    if (title === undefined) return { name, kind: "page" };
    const dayKey = pageIdentityKey(title);
    readKey?.(dayKey);
    return index?.linkRow(dayKey)?.link ?? { name: title, kind: "journal" };
  }
  // Older frozen publication rows encoded an identity opening by omitting link.
  if (row) return { name: row.target.kind === "absent" && !row.is_journal ? name : row.name, kind: row.is_journal ? "journal" : "page" };
  return { name, kind: absentLinkIsJournal(name) ? "journal" : "page" };
}
