/** GH #832 / #833: the Quick Capture window honours OG's own `config.edn` keys,
 * transcribed from `frontend.quick-capture/quick-capture` (logseq/og
 * `src/main/frontend/quick_capture.cljs`). No Tine setting exists for them.
 *
 * - `:quick-capture-options` `:default-page` / `:insert-today?` choose where a
 *   capture without a typed page goes (`captureDestination`);
 * - `:quick-capture-options` `:redirect-page?` opens that page afterwards
 *   (`shouldRedirect`);
 * - `:quick-capture-templates` `:text` wraps the captured text (`templateOutline`),
 *   which is where a timestamp (`{time}`, `{date}`) comes from.
 *
 * Pure: the receiver supplies the config, the clock and the open route.
 *
 * Deliberate differences from OG (named in the batch receipt):
 * - OG's chain "no page, no default page: the page open in the app" is not
 *   copied. Tine's capture is a global-hotkey window; with no option set an
 *   untitled capture keeps landing in today's journal, as it always has.
 * - OG applies its built-in template (`**{time}** [[quick capture]]: {text}
 *   {url}`) to every capture. Tine's window has no default template: with no
 *   `:text` template configured the typed blocks go in unchanged.
 * - OG's special page name `TODAY` is an external-caller token; in the window a
 *   typed `TODAY` names a page, as before.
 * - A multi-block capture is one entry: the template replaces the FIRST top-level
 *   block's text (its children and the other top-level blocks follow unchanged),
 *   `{url}` is empty (the window has no link), and trailing whitespace is trimmed
 *   (as for shares, shareShape.ts). */
import type { OutlineNode } from "./editor/outline";
import { quickCaptureContent } from "./shareShape";
import type { Format } from "./types";

/** OG's page when journals are disabled (`quick-capture`'s `:else`). */
export const QUICK_CAPTURE_PAGE = "quick capture";

export interface QuickCaptureOptions {
  /** `:quick-capture-options {:default-page …}`. */
  defaultPage?: string | null;
  /** `:quick-capture-options {:insert-today? …}`. */
  insertToday?: boolean;
  /** `:quick-capture-options {:redirect-page? …}`. */
  redirectPage?: boolean;
  /** `:quick-capture-templates {:text …}`. */
  textTemplate?: string | null;
  /** `:feature/enable-journals?`; absent means enabled, as in OG. */
  enableJournals?: boolean;
}

/** Where a capture lands. `journal` is today's journal; `page` a named page
 *  (created when missing). */
export type CaptureDestination = { kind: "journal" } | { kind: "page"; name: string };

/** The page a capture goes to, in OG's order (`quick-capture`'s `page` cond):
 * a typed title; today's journal when `insert-today?` (journals on); the
 * default page; today's journal; and `quick capture` when journals are off. */
export function captureDestination(typedTitle: string | null | undefined, options: QuickCaptureOptions): CaptureDestination {
  const typed = typedTitle?.trim();
  if (typed) return { kind: "page", name: typed };
  const journals = options.enableJournals !== false;
  if (journals && options.insertToday === true) return { kind: "journal" };
  const configured = options.defaultPage?.trim();
  if (configured) return { kind: "page", name: configured };
  return journals ? { kind: "journal" } : { kind: "page", name: QUICK_CAPTURE_PAGE };
}

/** The name of the page a destination resolves to. */
export function destinationName(destination: CaptureDestination, todayTitle: string): string {
  return destination.kind === "journal" ? todayTitle : destination.name;
}

/** OG redirects (`page-handler/create!` `:redirect?`) only when the capture goes
 * to a page other than the one open. `current` is the open page's name, if any. */
export function shouldRedirect(options: QuickCaptureOptions, destinationPage: string, current: string | null | undefined): boolean {
  return options.redirectPage === true && destinationPage.toLowerCase() !== (current ?? "").toLowerCase();
}

export interface TemplateContext {
  /** `date/get-current-time`. */
  time: string;
  /** `date/today`: today's journal title. */
  date: string;
  format: Format;
}

/** Apply the configured `:text` template to a parsed capture, or return `nodes`
 * untouched: no template (absent, empty or not a string), nothing captured, or a
 * template that comes out blank all keep today's behaviour. */
export function templateOutline(nodes: OutlineNode[], options: QuickCaptureOptions, context: TemplateContext): OutlineNode[] {
  const template = options.textTemplate;
  if (typeof template !== "string" || !template || !nodes.length) return nodes;
  const [first, ...rest] = nodes;
  const shaped = quickCaptureContent({ content: first.raw }, { ...context, textTemplate: template }).replace(/\s+$/, "");
  if (!shaped.trim()) return nodes;
  return [{ raw: shaped, children: first.children }, ...rest];
}
