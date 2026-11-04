// The published site's name answers (CM step 2, census Q13;
// docs/contracts/quick-share.md, "Published answers"). A snapshot's
// `answers: {v: 1, ...}` holds the live answerers' answers over the exported
// pages, filled by Rust at publish time (`published_answers.rs`): the
// inventory rows, the wrong-kind `resolve` answers, the alias groups and the
// journal formats and content days. This module admits that field and answers
// every name question by lookup in it, through the same row readers the app
// uses (`inventoryRows.ts`). A snapshot without a recognized `answers` (written
// before it existed, or by a later writer with another `v`) is answered by
// `upgradeLegacySnapshot`, the one declared second computer.
import type { AliasGroup, PageEntry, PageInventoryEntry, PageKind, ResolvedPage } from "./types";
import type { PublishedSnapshot } from "./publishedBackend";
import { pageIdentityKey } from "./pageIdentity";
import { indexInventoryRows, linkTargetIn, ownerPath, type InventoryRowIndex } from "./inventoryRows";
import { upgradeLegacySnapshot } from "./publishedLegacy";

type PublishedPage = PublishedSnapshot["pages"][number];

/** The `answers.v` this viewer reads. Its meaning is frozen: every future
 *  generation-1 viewer reads `v: 1` exactly so. */
export const PUBLISHED_ANSWERS_VERSION = 1;

export interface PublishedResolveAnswer { name: string; kind: PageKind; target: ResolvedPage }
export interface PublishedAliasGroupAnswer { name: string; aliases: string[]; pages: string[] }

/** `answers` version 1, as `published_answers.rs` writes it. */
export interface PublishedAnswersV1 {
  v: 1;
  /** `WholeGraph::inventory(All)` rows of the build graph, in its order, plus
   *  the synthetic home page (path `""`) of a multi-page publication. */
  inventory: PageInventoryEntry[];
  /** `WholeGraph::resolve(name, kind)` for a listed name asked as the kind its
   *  row does not answer, only where that is a file or an alias. */
  resolve: PublishedResolveAnswer[];
  /** `page_alias_group` of every exported page whose group is not empty. */
  alias_groups: PublishedAliasGroupAnswer[];
  journals: { title_format: string; file_name_format: string; content_days: number[] };
}

/** Every name question the published backend asks, answered by one object per
 *  snapshot: the v1 tables, or the legacy upgrade. */
export interface PublishedAnswerer {
  /** `pageInventory()` rows. */
  inventory(): PageInventoryEntry[];
  /** `resolvePage(name, kind)`. */
  resolve(name: string, kind: PageKind): ResolvedPage;
  /** The page `getPage(name, kind)` reads, or null. */
  page(name: string, kind: PageKind): PublishedPage | null;
  /** The page a journal feed row reads, or null. */
  feedPage(entry: PageEntry): PublishedPage | null;
  /** The page a public permalink `#…/page/<name>` opens, or null (missing). */
  permalinkPage(name: string): PublishedPage | null;
  /** The name whose baked backlinks `getBacklinks(name)` reads. */
  backlinksName(name: string): string;
  /** `pageAliasGroup(name)`. */
  aliasGroup(name: string): AliasGroup;
  /** `journalContentDays()`. */
  journalContentDays(): number[];
  /** `loadGraph().meta`'s journal formats. */
  journalFormats(): { title: string; file: string };
}

/** A recognized `answers` field that does not hold what its version says.
 *  In-scope threat: malformed input (a truncated or corrupted share or
 *  snapshot file, a defective writer); the viewer refuses it as damaged
 *  instead of silently falling back to the legacy reading. */
export class DamagedAnswersError extends Error {
  constructor() {
    super("This snapshot's published answers are damaged.");
  }
}

const isString = (v: unknown): v is string => typeof v === "string";
const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const isKind = (v: unknown): v is PageKind => v === "page" || v === "journal";
const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every(isString);
const isOptionalBool = (v: unknown) => v === undefined || typeof v === "boolean";
const isDay = (v: unknown) => typeof v === "number" && Number.isSafeInteger(v);

function targetShape(v: unknown, paths: ReadonlySet<string>): boolean {
  if (!isObject(v)) return false;
  switch (v.kind) {
    case "existing":
      return isString(v.id) && paths.has(v.id) && isStrings(v.others) && v.others.every((id) => paths.has(id));
    case "alias":
      return isStrings(v.owners) && v.owners.length > 0 && v.owners.every((id) => paths.has(id));
    case "absent":
      return isString(v.id);
    default:
      return false;
  }
}

function rowShape(v: unknown, paths: ReadonlySet<string>): boolean {
  return isObject(v) && isString(v.key) && isString(v.name) && typeof v.is_journal === "boolean"
    && (v.day === null || v.day === undefined || isDay(v.day)) && targetShape(v.target, paths)
    && (v.link === undefined || (isObject(v.link) && isString(v.link.name) && isKind(v.link.kind)))
    && isOptionalBool(v.namespace_parent) && isOptionalBool(v.empty_alias);
}

/** Admit a snapshot's `answers`: the v1 tables when they are present, of
 *  version 1 and well formed; `null` when absent or of a version this viewer
 *  does not read (the legacy reading then applies). A recognized v1 that is
 *  malformed, or names a file the snapshot does not hold, throws
 *  `DamagedAnswersError`. Unknown keys inside are ignored, as everywhere in a
 *  snapshot. O(table rows). */
export function admitPublishedAnswers(snapshot: { answers?: unknown; pages?: unknown }): PublishedAnswersV1 | null {
  const answers = snapshot.answers;
  if (answers === undefined) return null;
  if (!isObject(answers)) throw new DamagedAnswersError();
  if (answers.v !== PUBLISHED_ANSWERS_VERSION) return null;
  const paths = new Set(
    Array.isArray(snapshot.pages)
      ? snapshot.pages.flatMap((page) => (isObject(page) && isString(page.path) ? [page.path] : []))
      : [],
  );
  const journals = answers.journals;
  const ok = Array.isArray(answers.inventory) && answers.inventory.every((row) => rowShape(row, paths))
    && Array.isArray(answers.resolve) && answers.resolve.every((entry) =>
      isObject(entry) && isString(entry.name) && isKind(entry.kind) && targetShape(entry.target, paths))
    && Array.isArray(answers.alias_groups) && answers.alias_groups.every((group) =>
      isObject(group) && isString(group.name) && isStrings(group.aliases) && isStrings(group.pages))
    && isObject(journals) && isString(journals.title_format) && isString(journals.file_name_format)
    && Array.isArray(journals.content_days) && journals.content_days.every(isDay);
  if (!ok) throw new DamagedAnswersError();
  return answers as unknown as PublishedAnswersV1;
}

/** Answer every name question from the v1 tables. O(rows) to index, O(1) per
 *  question. */
function answersV1(snapshot: PublishedSnapshot, answers: PublishedAnswersV1): PublishedAnswerer {
  const rows: InventoryRowIndex = indexInventoryRows(answers.inventory);
  const resolved = new Map<string, ResolvedPage>();
  for (const entry of answers.resolve) resolved.set(`${entry.kind}\0${pageIdentityKey(entry.name)}`, entry.target);
  const groups = new Map<string, AliasGroup>();
  for (const group of answers.alias_groups) {
    const key = pageIdentityKey(group.name);
    if (!groups.has(key)) groups.set(key, { aliases: group.aliases, pages: group.pages });
  }
  const byPath = new Map<string, PublishedPage>();
  for (const page of snapshot.pages) if (!byPath.has(page.path)) byPath.set(page.path, page);
  const resolve = (name: string, kind: PageKind): ResolvedPage => {
    const key = pageIdentityKey(name);
    return (kind === "journal" ? rows.journals : rows.pages).get(key)?.target
      ?? resolved.get(`${kind}\0${key}`)
      ?? { kind: "absent", id: "" };
  };
  const page = (name: string, kind: PageKind): PublishedPage | null => {
    const path = ownerPath(resolve(name, kind));
    return path === undefined ? null : byPath.get(path) ?? null;
  };
  return {
    inventory: () => answers.inventory,
    resolve,
    page,
    feedPage: (entry) => byPath.get(entry.path) ?? null,
    permalinkPage(name) {
      // What `[[name]]` opens (the row's `link`, the store's `link_target`);
      // a name no row lists, or one that opens no file, is missing.
      const key = pageIdentityKey(name);
      if (!rows.pages.has(key) && !rows.journals.has(key)) return null;
      const destination = linkTargetIn(rows, name);
      return page(destination.name, destination.kind);
    },
    backlinksName(name) {
      return page(name, "page")?.name ?? name;
    },
    aliasGroup: (name) => structuredClone(groups.get(pageIdentityKey(name)) ?? { aliases: [], pages: [] }),
    journalContentDays: () => [...answers.journals.content_days],
    journalFormats: () => ({ title: answers.journals.title_format, file: answers.journals.file_name_format }),
  };
}

const answerers = new WeakMap<PublishedSnapshot, PublishedAnswerer>();

/** The snapshot's answerer, built once per snapshot: the v1 tables when
 *  admitted, else the legacy upgrade. Throws `DamagedAnswersError` for a
 *  damaged recognized `answers` (the loaders admit it first). */
export function publishedAnswerer(snapshot: PublishedSnapshot): PublishedAnswerer {
  let answerer = answerers.get(snapshot);
  if (!answerer) {
    const answers = admitPublishedAnswers(snapshot);
    answerer = answers ? answersV1(snapshot, answers) : upgradeLegacySnapshot(snapshot);
    answerers.set(snapshot, answerer);
  }
  return answerer;
}
