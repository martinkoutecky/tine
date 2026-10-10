/** S2 of the native-integrations batch (ADR 0073): the block a shared item
 * becomes, transcribed from OG (logseq/og 6e7afa8eb):
 * - `frontend/mobile/intent.cljs`: `is-link`, `extract-highlight`,
 *   `transform-args` (Android text, v1), `embed-asset-file` (a lone media
 *   file, v1) and `handle-payload` (iOS share sheet, v2);
 * - `frontend/quick_capture.cljs` `quick-capture` (the link shape: video
 *   embed, tweet embed, raw URL or a titled link);
 * - `frontend/util/text.cljs` (the video URL patterns).
 * Pure: the caller supplies time, date, format, templates and asset links.
 *
 * Deliberate differences (named in the batch receipt):
 * - one shaping per item instead of one per producer API: an item with no
 *   files takes OG's text path (`transform-args` + `quick-capture`) on every
 *   platform, so a shared YouTube link becomes a video embed on iOS too, where
 *   OG's v2 path left the raw URL;
 * - asset links use Tine's existing asset markup (`assetMarkdown`), which
 *   gives images an empty label, as every other Tine asset insert does;
 * - trailing whitespace is trimmed (OG's default template leaves "{url}"'s
 *   separating space behind when there is no link). */
import type { Format } from "./types";

/** OG defaults (`[:quick-capture-templates :text]` / `:media`). */
export const OG_TEXT_TEMPLATE = "**{time}** [[quick capture]]: {text} {url}";
export const OG_MEDIA_TEMPLATE = "**{time}** [[quick capture]]: {url}";

// frontend/util/text.cljs
const YOUTUBE = /^((?:https?:)?\/\/)?((?:www|m).)?((?:youtube.com|youtu.be|y2u.be|youtube-nocookie.com))(\/(?:[\w-]+\?v=|embed\/|v\/)?)([\w-]+)([\S^\?]+)?$/;
const LOOM = /^((?:https?:)?\/\/)?((?:www).)?((?:loom.com))(\/(?:share\/|embed\/))([\w-]+)(\S+)?$/;
const VIMEO = /^((?:https?:)?\/\/)?((?:www).)?((?:player.vimeo.com|vimeo.com))(\/(?:video\/)?)([\w-]+)(\S+)?$/;
const BILIBILI = /^((?:https?:)?\/\/)?((?:www).)?((?:bilibili.com))(\/(?:video\/)?)([\w-]+)(\?p=(\d+))?(\S+)?$/;

/** `text-util/get-matched-video`. */
export function isVideoUrl(url: string): boolean {
  return !!url && [YOUTUBE, LOOM, VIMEO, BILIBILI].some((pattern) => pattern.test(url));
}

/** `quick-capture` `is-tweet-link`. */
export function isTweetLink(url: string): boolean {
  return !!url && (/^https:\/\/twitter\.com\/.*?\/status\/.*?$/.test(url) || /^https:\/\/x\.com\/.*?\/status\/.*?$/.test(url));
}

/** `intent` `is-link` (`re-matches`: the whole string). */
export function isLink(url: string | null | undefined): boolean {
  return !!url && /^[a-zA-Z0-9]+:\/\/.*$/.test(url);
}

/** clojure.string/replace with a string match: every occurrence, literally. */
function replaceAll(text: string, match: string, replacement: string): string {
  return text.split(match).join(replacement);
}

/** goog.string/stripQuotes with one quote character. */
function stripQuotes(text: string, quote: string): string {
  return text.length > 1 && text.startsWith(quote) && text.endsWith(quote) ? text.slice(1, -1) : text;
}

/** `intent` `extract-highlight`: a browser share may prefix the URL with the
 * highlighted text. Returns `[highlight, link]`. */
export function extractHighlight(url: string): [string | null, string | null] {
  const link = /\s+([a-zA-Z0-9]+:\/\/[\S]*)$/.exec(url)?.[1] ?? "";
  const highlight = link ? stripQuotes(replaceAll(url, link, "").trimEnd(), "\"") : "";
  if (highlight) return [highlight, link];
  if (isLink(url)) return [null, url];
  return [url, null];
}

export interface CaptureArgs { url?: string | null; title?: string | null; content?: string | null }

/** `intent` `transform-args`. */
export function transformArgs(args: CaptureArgs): CaptureArgs {
  if (isLink(args.url)) return args;
  const [highlight, url] = extractHighlight(args.url ?? "");
  return { ...args, url, content: highlight };
}

/** `config/link-format`. */
function linkFormat(format: Format, label: string | null, link: string): string {
  if (!label) return link;
  return format === "org" ? `[[${link}][${label}]]` : `[${label}](${link})`;
}

export interface ShapeContext {
  /** `date/get-current-time`. */
  time: string;
  /** `date/today`: the journal title. */
  date: string;
  format: Format;
  textTemplate?: string | null;
  mediaTemplate?: string | null;
}

/** `quick-capture`'s content (the page choice and editor insertion are the
 * caller's: a share always lands at the bottom of today's journal). */
export function quickCaptureContent(args: CaptureArgs, context: ShapeContext): string {
  const title = args.title ?? "";
  const url = args.url ?? "";
  const text = args.content?.trim() || "";
  let link: string;
  if (!url.trim()) link = title;
  else if (isVideoUrl(url)) link = `${title} {{video ${url}}}`;
  else if (isTweetLink(url)) link = `{{twitter ${url}}}`;
  else if (title === url) link = linkFormat(context.format, null, url);
  else link = linkFormat(context.format, title, url);
  let content = context.textTemplate || OG_TEXT_TEMPLATE;
  content = replaceAll(content, "{time}", context.time);
  content = replaceAll(content, "{date}", context.date);
  content = replaceAll(content, "{url}", link);
  return replaceAll(content, "{text}", text);
}

/** `embed-asset-file`'s content for one media file link. */
export function mediaContent(assetLink: string, context: ShapeContext): string {
  let content = context.mediaTemplate || OG_MEDIA_TEMPLATE;
  content = replaceAll(content, "{time}", context.time);
  content = replaceAll(content, "{date}", context.date);
  content = replaceAll(content, "{text}", "");
  return replaceAll(content, "{url}", assetLink);
}

/** `handle-payload`'s content: text plus the rich parts (a web link as its
 * URL, a file as its asset link) one per line; null when both are empty. */
export function payloadContent(text: string, rich: string[], context: ShapeContext): string | null {
  const richContent = rich.join("\n");
  if (!text && !richContent) return null;
  let content = context.textTemplate || OG_TEXT_TEMPLATE;
  content = replaceAll(content, "{time}", context.time);
  content = replaceAll(content, "{date}", context.date);
  content = replaceAll(content, "{text}", text);
  return replaceAll(content, "{url}", richContent);
}

export interface SharedContent {
  text?: string | null;
  title?: string | null;
  url?: string | null;
  /** Asset links of the item's files, already imported into the graph. */
  assets: string[];
}

/** The block content for one shared item, or null when it carries nothing. */
export function shapeShare(item: SharedContent, context: ShapeContext): string | null {
  const text = item.text?.trim() ? item.text : null;
  const url = item.url?.trim() ? item.url : null;
  let content: string | null;
  if (!item.assets.length) {
    if (!text && !url) return null;
    const args = transformArgs({ url: url ?? text, title: item.title ?? null });
    content = quickCaptureContent(url && text ? { ...args, content: text } : args, context);
  } else if (item.assets.length === 1 && !text && !url) {
    content = mediaContent(item.assets[0], context);
  } else {
    content = payloadContent(text ?? "", [...(url ? [url] : []), ...item.assets], context);
  }
  const trimmed = content?.replace(/\s+$/, "") ?? "";
  return trimmed.trim() ? trimmed : null;
}

/** `date/get-current-time`: the locale's two-digit 24-hour time. */
export function captureTime(date: Date, locale?: string): string {
  return date.toLocaleTimeString(locale ?? (typeof navigator === "undefined" ? undefined : navigator.language),
    { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

/** One block of outline Markdown holding `content` (continuation lines
 * indented under its bullet), the form `appendToTodayJournal` takes. */
export function asOutlineBlock(content: string): string {
  return `- ${content.split("\n").join("\n  ")}`;
}
