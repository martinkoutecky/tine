// Tine's UI language (tine#107; docs/contracts/localization.md). The frontend is
// the only formatter: messages live in src/i18n/locales/<tag>/*.ftl (Fluent),
// English is complete, every other locale falls back to English message by
// message. Graph content (page titles, journal titles, properties, markers,
// Markdown, query syntax, plugin text) is never translated.
//
// Two value kinds cross module boundaries: `Msg` (a catalog id + arguments,
// resolved where it is shown, so a language change re-renders it) and `RawText`
// (text that is not ours to translate: plugin-authored, OS-originated). Tables
// and stored state hold these, never a string `t()` produced at module load.
import { FluentBundle, FluentResource, type FluentVariable } from "@fluent/bundle";
import { createSignal } from "solid-js";
import { LOCALES, type MessageArgs, type MessageId } from "./messages.gen";

export type { MessageId } from "./messages.gen";
export { LOCALES } from "./messages.gen";

type Args<I extends MessageId> = keyof MessageArgs[I] extends never ? [] : [MessageArgs[I]];

export interface Msg {
  readonly kind: "msg";
  readonly id: MessageId;
  readonly args?: Readonly<Record<string, string | number>>;
}
export interface RawText {
  readonly kind: "raw";
  readonly text: string;
}
/** Text the UI shows: a catalog message, or text that is not ours to translate. */
export type UiText = Msg | RawText;

/** A catalog message, resolved when shown. */
export function msg<I extends MessageId>(id: I, ...args: Args<I>): Msg {
  return args.length ? { kind: "msg", id, args: args[0] as Record<string, string | number> } : { kind: "msg", id };
}

/** Text from outside the catalog (a plugin, the OS, the user's graph): shown as is. */
export function rawText(text: string): RawText {
  return { kind: "raw", text };
}

export const SOURCE_LOCALE = "en";

const englishFiles = import.meta.glob("./locales/en/*.ftl", {
  query: "?raw", import: "default", eager: true,
}) as Record<string, string>;
const otherFiles = import.meta.glob(["./locales/*/*.ftl", "!./locales/en/*.ftl"], {
  query: "?raw", import: "default",
}) as Record<string, () => Promise<string>>;

function bundleOf(tag: string, sources: readonly string[]): FluentBundle {
  // No Unicode isolation marks: no shipped locale is right-to-left, and the
  // invisible FSI/PDI characters leak into copied text and test assertions.
  const bundle = new FluentBundle(tag, { useIsolating: false });
  for (const source of sources) {
    const errors = bundle.addResource(new FluentResource(source));
    // Fixed text (I-5); scripts/lib/i18n-catalog.mjs checkCatalog (the catalog test) names it.
    if (errors.length) console.error("i18n: a catalog failed to parse");
  }
  return bundle;
}

const bundles = new Map<string, FluentBundle>([[SOURCE_LOCALE, bundleOf(SOURCE_LOCALE, Object.values(englishFiles))]]);
const [locale, setLocaleSignal] = createSignal(SOURCE_LOCALE);

/** Reactive: the active UI locale's tag (`"en"`, `"zh-CN"`). */
export const activeLocale = locale;

const loading = new Map<string, Promise<void>>();
/** Load `tag`'s catalog once (English is always loaded). Concurrent calls share
 * one load; a failed load is forgotten, so a later call retries it. */
export function loadLocale(tag: string): Promise<void> {
  if (bundles.has(tag)) return Promise.resolve();
  const inFlight = loading.get(tag);
  if (inFlight) return inFlight;
  const prefix = `./locales/${tag}/`;
  const files = Object.entries(otherFiles).filter(([file]) => file.startsWith(prefix));
  if (!files.length) return Promise.reject(new Error(`no catalog for ${tag}`));
  const load = Promise.all(files.map(([, read]) => read())).then((sources) => {
    bundles.set(tag, bundleOf(tag, sources));
  }).finally(() => loading.delete(tag));
  loading.set(tag, load);
  return load;
}

/**
 * The shipped locale for a preference (`""` = the system's) and the system's
 * language list: an exact shipped tag, else the first requested tag a locale
 * serves (by full tag, then by its language subtag), else English.
 */
export function negotiateLocale(preference: string, requested: readonly string[]): string {
  const known = (tag: string) => LOCALES.find((entry) => entry.tag.toLowerCase() === tag.toLowerCase())?.tag;
  if (preference && known(preference)) return known(preference)!;
  const serving = (tag: string) => LOCALES.find((entry) =>
    entry.matches.some((match) => match.toLowerCase() === tag.toLowerCase()))?.tag;
  for (const tag of requested) {
    const found = serving(tag) ?? serving(tag.split("-")[0]);
    if (found) return found;
  }
  return SOURCE_LOCALE;
}

/** Show `tag` now: its catalog must be loaded (`loadLocale`). Which activation
 * may call this is the caller's decision (preference.ts owns the UI language). */
export function showLocale(tag: string): void {
  if (!bundles.has(tag)) throw new Error(`catalog for ${tag} is not loaded`);
  setLocaleSignal(tag);
  if (typeof document !== "undefined") document.documentElement.lang = tag;
}

/** Load `tag`'s catalog, then show it (tests and single-owner callers; the UI
 * language goes through preference.ts, which owns concurrent activations). */
export async function setActiveLocale(tag: string): Promise<void> {
  await loadLocale(tag);
  showLocale(tag);
}

const reported = new Set<string>();
/** One console line per failing message (fixed text, I-5); the catalog test names it. */
function reportOnce(key: string): void {
  if (reported.has(key)) return;
  reported.add(key);
  console.error("i18n: a message failed to format; showing English");
}

function formatIn(bundle: FluentBundle, id: string, args?: Readonly<Record<string, FluentVariable>>): string | null {
  const [name, attribute] = id.split(".");
  const message = bundle.getMessage(name);
  const pattern = attribute ? message?.attributes[attribute] : message?.value;
  if (!pattern) return null;
  const errors: Error[] = [];
  const text = bundle.formatPattern(pattern, args as Record<string, FluentVariable> | undefined, errors);
  // A salvaged string with errors is not shown: the English message is (review #5).
  if (errors.length) {
    reportOnce(`${bundle.locales[0]}:${id}`);
    return null;
  }
  return text;
}

function format(tag: string, id: MessageId, args?: Readonly<Record<string, string | number>>): string {
  const chosen = bundles.get(tag);
  const text = (chosen && tag !== SOURCE_LOCALE ? formatIn(chosen, id, args) : null)
    ?? formatIn(bundles.get(SOURCE_LOCALE)!, id, args);
  if (text !== null) return text;
  reportOnce(`en:${id} did not format`);
  return id;
}

/** Reactive: message `id` in the active locale (English where it has none). */
export function t<I extends MessageId>(id: I, ...args: Args<I>): string {
  return format(locale(), id, args[0] as Record<string, string | number> | undefined);
}

/** Reactive: `text` as the UI shows it. */
export function render(text: UiText): string {
  return text.kind === "raw" ? text.text : format(locale(), text.id, text.args);
}

/** `text` in English, for bug reports, dedupe keys and session history (G5). */
export function renderEnglish(text: UiText): string {
  return text.kind === "raw" ? text.text : format(SOURCE_LOCALE, text.id, text.args);
}

/** A stable key for `text`: the message id and English rendering, or the raw text. */
export function textKey(text: UiText): string {
  return text.kind === "raw" ? `raw:${text.text}` : `${text.id}:${renderEnglish(text)}`;
}
