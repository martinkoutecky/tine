// Settings search vocabulary and matching contract: each visible setting has
// one label/description and optionally aliases; search matches all terms within
// that entry. Cost O(entries × query length), no I/O or failure.
import { render, renderEnglish, type Msg } from "../i18n";
import { richPlain } from "../i18n/Rich";
import type { SettingsTabId } from "../ui";
type Tab = SettingsTabId;

/** Mark visible query terms without changing matching or interpreting label HTML.
 * O(text length × query terms); overlapping matches form one highlighted span. */
export function settingHighlight(text: string, query: string): { text: string; marked: boolean }[] {
  const marked = new Uint8Array(text.length);
  const folded = text.toLowerCase();
  // Some Latin letters expand when lowercased (İ → i + combining dot).
  // Keep folded offsets attached to their original visible characters.
  const starts: number[] = [];
  const ends: number[] = [];
  let offset = 0;
  for (const char of text) {
    for (let i = 0; i < char.toLowerCase().length; i++) {
      starts.push(offset);
      ends.push(offset + char.length);
    }
    offset += char.length;
  }
  for (const term of query.toLowerCase().trim().split(/\s+/).filter(Boolean)) {
    let start = folded.indexOf(term);
    while (start >= 0) {
      marked.fill(1, starts[start], ends[start + term.length - 1]);
      start = folded.indexOf(term, start + 1);
    }
  }
  const parts: { text: string; marked: boolean }[] = [];
  for (let start = 0, end = 0; start < text.length; start = end) {
    end = start + 1;
    while (end < text.length && marked[end] === marked[start]) end++;
    parts.push({ text: text.slice(start, end), marked: !!marked[start] });
  }
  return parts;
}

import type { SettingDescriptor } from "./settingsDescriptor";
export type SettingSearchEntry = Omit<SettingDescriptor, "id">;
import { settingEntries } from "./settingsRegistry";

function textsOf(...parts: (string | Msg | undefined)[]): string {
  return parts.flatMap((text) => text === undefined ? [] : typeof text === "string" ? [text] : [richPlain(render(text)), richPlain(renderEnglish(text))]).join(" ").toLowerCase();
}

/** A one-character term (any script) matches only text shown in the active
 * language: label, description and keywords, not their English originals or the
 * aliases. A single "S" would otherwise hit almost every hidden English label in
 * the Chinese interface, while a single "发" must still find descriptions
 * (GH #706). Longer terms match both languages. */
const isShortTerm = (term: string) => [...term].length === 1;
const queryTerms = (query: string) => query.toLowerCase().trim().split(/\s+/).filter(Boolean);

function shownTextsOf(...parts: (string | Msg | undefined)[]): string {
  return parts.flatMap((text) => text === undefined ? [] : [typeof text === "string" ? text : richPlain(render(text))]).join(" ").toLowerCase();
}

/** Whether every query word hits the entry's label, keywords or aliases. */
function nameMatches(entry: SettingSearchEntry, terms: string[]): boolean {
  const names = textsOf(entry.label, entry.keywords, ...(entry.aliases ?? []));
  const shownNames = shownTextsOf(entry.label, entry.keywords);
  return terms.every((term) => (isShortTerm(term) ? shownNames : names).includes(term));
}

/** Match all query words against one entry's label, description, keywords and
 * aliases in both languages; a one-character word only against the shown
 * language's label, description and keywords. O(entry text × query words), no
 * I/O or failure. */
export function settingMatches(entry: SettingSearchEntry, query: string): boolean {
  const terms = queryTerms(query);
  const haystack = textsOf(entry.label, entry.description, entry.searchDescription, entry.keywords, ...(entry.aliases ?? []));
  const shown = shownTextsOf(entry.label, entry.description, entry.searchDescription, entry.keywords);
  return terms.every((term) => (isShortTerm(term) ? shown : haystack).includes(term));
}

/** Explain query words absent from the visible label, including hits in hidden
 * keywords, aliases and English originals. Ranking still uses nameMatches;
 * that group includes hidden text and cannot decide whether evidence is needed. */
export function matchedDescription(entry: SettingSearchEntry, query: string): string | undefined {
  let missing = queryTerms(query).filter(term => !shownTextsOf(entry.label).includes(term));
  if (!missing.length) return undefined;
  const plain = (value: string | Msg, english = false) => typeof value === "string" ? value : richPlain(english ? renderEnglish(value) : render(value));
  const sources = [
    ...[entry.description, entry.searchDescription, entry.keywords].flatMap(value => value === undefined ? [] : [{ text: plain(value), shown: true }]),
    ...[entry.label, entry.description, entry.searchDescription, entry.keywords, ...(entry.aliases ?? [])].flatMap(value => value === undefined ? [] : [{ text: plain(value, true), shown: false }]),
  ];
  const snippets: string[] = [];
  for (const { text, shown } of sources) {
    // One-character terms must obey settingMatches' shown-language rule.
    // One source can need several excerpts when its matching words are far apart.
    while (true) {
      const hits = missing.filter(term => shown || !isShortTerm(term))
        .map(term => ({ term, at: text.toLowerCase().indexOf(term) })).filter(hit => hit.at >= 0)
        .sort((a, b) => a.at - b.at);
      if (!hits.length) break;
      const snippet = excerpt(text, hits[0].at, hits[0].term.length);
      snippets.push(snippet);
      missing = missing.filter(term => !snippet.toLowerCase().includes(term));
    }
    if (!missing.length) break;
  }
  return snippets.length ? snippets.join(" · ") : undefined;
}

/** About one line of `text` around position `at`, with an ellipsis where cut, so a
 * long description shows its match instead of its opening sentences. */
function excerpt(text: string, at: number, matchLength: number): string {
  const chars = [...text];
  // Search offsets refer to lowercased UTF-16 text; map them back before slicing
  // visible code points (lowercasing can expand a character, e.g. İ).
  let index = 0, matchEnd = 0, foldedOffset = 0;
  for (const char of chars) {
    if (foldedOffset <= at) index = matchEnd;
    if (foldedOffset >= at + matchLength) break;
    foldedOffset += char.toLowerCase().length;
    matchEnd++;
  }
  let start = Math.max(0, index - 40);
  // Start at a word boundary when the text has one before the match (Latin text; CJK has none).
  const space = chars.slice(start, index).findIndex((char) => /\s/.test(char));
  if (start > 0 && space >= 0) start += space + 1;
  const end = Math.min(chars.length, Math.max(start + 120, matchEnd));
  return `${start > 0 ? "…" : ""}${chars.slice(start, end).join("").trim()}${end < chars.length ? "…" : ""}`;
}

/** Matching entries with label/keyword/alias matches before description-only
 * matches; the given order is kept within each group. O(entries × text × words). */
export function rankedSettingMatches<T extends SettingSearchEntry>(entries: readonly T[], query: string): T[] {
  const terms = queryTerms(query);
  const hits = entries.filter((entry) => settingMatches(entry, query));
  return [...hits.filter((entry) => nameMatches(entry, terms)), ...hits.filter((entry) => !nameMatches(entry, terms))];
}

/** Whether a tab has an advanced setting matching a nonempty query. Used to
 * reveal that section while searching. O(entries × query length), no I/O. */
export function advancedMatch(tab: Tab, query: string): boolean {
  return !!query.trim() && settingEntries().some((entry) => entry.tab === tab && entry.level === "advanced" && settingMatches(entry, query));
}
