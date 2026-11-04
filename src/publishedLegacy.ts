// I-12 EXCEPTION (declared; CM step 2, census Q13; docs/contracts/quick-share.md,
// "Published answers"): generation-1 snapshots written before `answers`
// existed carry no Rust answer; the contract keeps them readable forever.
// Retire with `/s/1/`.
//
// `upgradeLegacySnapshot` answers such a snapshot (and one whose `answers.v`
// this viewer does not read) exactly as the published viewer did before the
// `answers` field: the name-to-page lookup by page identity key with the
// snapshot's one-owner `aliases` pairs as the fallback, the inventory as the
// exported entries only, no alias group, every exported journal as a content
// day, the default journal formats, and a permalink that opens a page of
// either kind. It is that logic moved, not improved, so every existing share
// keeps showing what it shows; the frozen generation-1 fixtures pin it
// (`publishedLegacy.test.ts`). Nothing else may compute a name answer from a
// snapshot (`publishedBackend.ts` and `publishedPermalink.ts` ask the
// answerer only).
import type { PageInventoryEntry, PageKind, ResolvedPage } from "./types";
import type { PublishedSnapshot } from "./publishedBackend";
import type { PublishedAnswerer } from "./publishedAnswers";
import { pageIdentityKey } from "./pageIdentity";
import { DEFAULT_TITLE_FORMAT } from "./journal";

type PublishedPage = PublishedSnapshot["pages"][number];

/** The journal formats the legacy viewer always claimed. */
const LEGACY_TITLE_FORMAT = DEFAULT_TITLE_FORMAT;
const LEGACY_FILE_NAME_FORMAT = "yyyy_MM_dd";

export function upgradeLegacySnapshot(snapshot: PublishedSnapshot): PublishedAnswerer {
  const pageByName = (name: string): PublishedPage | null => {
    const wanted = pageIdentityKey(name);
    const direct = snapshot.pages.find((page) => pageIdentityKey(page.name) === wanted);
    if (direct) return direct;
    const alias = snapshot.aliases.find(([from]) => pageIdentityKey(from) === wanted);
    return alias ? snapshot.pages.find((page) => pageIdentityKey(page.name) === pageIdentityKey(alias[1])) ?? null : null;
  };
  const page = (name: string, kind: PageKind): PublishedPage | null => {
    const found = pageByName(name);
    return found && found.kind === kind ? found : null;
  };
  return {
    inventory: (): PageInventoryEntry[] => snapshot.entries.map((entry) => ({
      key: pageIdentityKey(entry.name), name: entry.name,
      is_journal: entry.kind === "journal", day: entry.date_key,
      target: { kind: "existing" as const, id: entry.path, others: [] },
    })),
    resolve(name, kind): ResolvedPage {
      const found = page(name, kind);
      return found ? { kind: "existing", id: found.path, others: [] } : { kind: "absent", id: "" };
    },
    page,
    feedPage: (entry) => pageByName(entry.name),
    permalinkPage(name) {
      const wanted = pageIdentityKey(name);
      let found = snapshot.pages.find((candidate) => pageIdentityKey(candidate.name) === wanted);
      if (!found) {
        const alias = snapshot.aliases.find(([from]) => pageIdentityKey(from) === wanted);
        if (alias) found = snapshot.pages.find((candidate) => pageIdentityKey(candidate.name) === pageIdentityKey(alias[1]));
      }
      return found ?? null;
    },
    backlinksName: (name) => name,
    aliasGroup: () => ({ aliases: [], pages: [] }),
    journalContentDays: () => snapshot.entries
      .filter((entry) => entry.kind === "journal" && entry.date_key !== null)
      .map((entry) => entry.date_key as number),
    journalFormats: () => ({ title: LEGACY_TITLE_FORMAT, file: LEGACY_FILE_NAME_FORMAT }),
  };
}
