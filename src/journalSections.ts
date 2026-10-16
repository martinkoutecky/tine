// Today's sections page (GH #605, design 2026-10-14): an ordinary page marked
// `tine/journal-sections:: true` defines what Tine shows under today's journal.
// If the page exists it wins; otherwise config.edn `:default-queries` and the
// built-in agenda render exactly as before. Precedence, not synchronisation:
// Tine never writes `:default-queries` back, and the page is seeded from it once,
// when the user creates it. Located by its marker through the query engine, so
// renaming the page keeps it working and no config key is written.
import type { BlockDto, Format, PageDto } from "./types";
import { backend } from "./backend";
import { graphMeta } from "./graphSession";
import { graphOwner, readOwned } from "./owned";
import { journalSectionsPage } from "./document";

export const JOURNAL_SECTIONS_PROPERTY = "tine/journal-sections";
export const DEFAULT_JOURNAL_SECTIONS_PAGE = "Journal sections";

/** The marker pre-block in the page's own format: Org reads a Markdown
 *  `key:: value` line as body text, not a property (same rule as Favorites). */
export const sectionsMarker = (format: Format) =>
  format === "org" ? `#+${JOURNAL_SECTIONS_PROPERTY}: true` : `${JOURNAL_SECTIONS_PROPERTY}:: true`;

/** Names of the pages carrying the marker, from the engine's page-property
 *  index (one evaluation, memoized per snapshot). */
export async function locateSectionsPages(): Promise<string[]> {
  const owner = graphOwner();
  const parsed = await readOwned(owner, backend().parseQuery(`(page-property ${JOURNAL_SECTIONS_PROPERTY} true)`, "og"));
  if (parsed.kind === "stale") return [];
  const result = await readOwned(owner, backend().queryRun(parsed.value.query, parsed.value.view));
  return result.kind === "current" && result.value.anchor === "page" ? result.value.pages.map((page) => page.name) : [];
}

export type SectionItem =
  | { kind: "upcoming"; raw: string; format: Format }
  | { kind: "query"; raw: string; format: Format }
  | { kind: "begin-query"; raw: string; format: Format };

/** Query and Upcoming blocks of the page in outline order, any depth. Other text is the
 *  user's notes for themselves and stays on the page. `classify` is the block
 *  renderer's own detection (detectMacro / inspectBeginQuery), injected so this
 *  module never re-derives query syntax. */
export function sectionItems(blocks: readonly BlockDto[], format: Format,
  classify: (raw: string, format: Format) => SectionItem["kind"] | null): SectionItem[] {
  const out: SectionItem[] = [];
  const visit = (nodes: readonly BlockDto[]) => {
    for (const block of nodes) {
      const kind = classify(block.raw, format);
      if (kind) out.push({ kind, raw: block.raw, format });
      visit(block.children);
    }
  };
  visit(blocks);
  return out;
}

/** The page Customize creates: the marker, one note, then the current
 *  effective sections in display order: each valid `:default-queries` entry
 *  as an OG `#+BEGIN_QUERY` block (Logseq reads it unchanged), then the
 *  agenda unless config.edn disables it. `agenda` is the agenda's TQL macro
 *  block text: its scheduled/deadline window has no `{{query}}` form (D-18). */
export function seedSectionsPage(name: string, format: Format, agenda: string): PageDto {
  const meta = graphMeta();
  const block = (raw: string): BlockDto => ({ id: "", raw, collapsed: false, children: [] });
  const queries = (meta?.default_journal_queries ?? []).flatMap((query) => query.seed ? [block(query.seed)] : []);
  const blocks = [
    block("Queries on this page show under today's journal in Tine, in this order. To show future journal references, add a separate block containing {{upcoming}}. Logseq keeps using :default-queries in config.edn. Delete this page to go back to config.edn."),
    ...queries,
    ...(meta?.disable_scheduled_and_deadline_query ? [] : [block(agenda)]),
  ];
  return journalSectionsPage(name, sectionsMarker(format), blocks);
}
