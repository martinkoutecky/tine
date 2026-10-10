/** External addresses are open-only: the parser admits exactly graph, page or
 * block navigation. Cost O(URL bytes); malformed/unrecognised inputs throw.
 * The graph UUID is authority; folder names never participate in resolution. */
export type TineLink = { graph?: string; page?: string; block?: string };
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const checked = (text: string) => { if (!UUID.test(text)) throw new Error("Invalid Tine link ID"); return text; };
/** Graph ids are minted lower-case, so the graph id is folded to that spelling. */
const uuid = (text: string) => checked(text).toLowerCase();
/** A block id is the AUTHORED `id::` spelling, and the store looks blocks up by
 * exact string, so an upper-case id imported from another tool must survive the
 * round trip unfolded (the Rust side validates the same shape without folding;
 * both are pinned by tests/fixtures/deep-link-ids.json). */
const blockId = checked;
export const graphLink = (id: string) => `tine://graph/${uuid(id)}`;
export const pageLink = (name: string, id: string) => `tine://page/${encodeURIComponent(name)}?graph=${uuid(id)}`;
export const blockLink = (id: string) => `tine://block/${blockId(id)}`;
/** Current-graph routes (S3 of the native integrations, ADR 0073): quick
 * actions, launcher shortcuts, App Intents and Spotlight results. Open-only
 * like every Tine address; `capture` starts editing an empty bottom block of
 * today's journal and writes nothing until the user types. */
export type AppRoute =
  | { route: "today" } | { route: "capture" }
  | { route: "search"; query: string } | { route: "page"; page: string };
/** An app route, or null when `text` is not one (it may still be a
 * known-graph address for {@link parseTineLink}). Malformed escapes throw. */
export function parseAppRoute(text: string): AppRoute | null {
  if (text.length > 8192 || /[\s\u0000-\u001f]/.test(text)) return null;
  const match = /^tine:\/\/(today|capture|search|page)(?:\/([^?#]*))?(?:\?([^#]*))?$/.exec(text);
  if (!match) return null;
  const [, route, path, query] = match;
  const params = new URLSearchParams(query ?? "");
  const keys = [...params.keys()];
  if (route === "page") return path && !keys.length && query === undefined ? { route, page: decodeURIComponent(path) } : null;
  if (path) return null;
  if (route === "search" && keys.every((key) => key === "q") && keys.length <= 1) return { route, query: params.get("q") ?? "" };
  if ((route === "today" || route === "capture") && query === undefined) return { route };
  return null;
}
export function parseTineLink(text: string): TineLink {
  const match = /^tine:\/\/(graph|page|block)\/([^?#]*)(?:\?([^#]*))?$/.exec(text);
  if (!match || text.length > 8192 || /[\s\u0000-\u001f]/.test(text)) throw new Error("Unsupported Tine link");
  // Read the raw encoded path: URL.pathname normalises dot-only page names.
  const value = decodeURIComponent(match[2]);
  const params = new URLSearchParams(match[3]);
  const keys = [...params.keys()];
  if (match[1] === "graph" && !keys.length) return { graph: uuid(value) };
  if (match[1] === "block" && !keys.length) return { block: blockId(value) };
  if (match[1] === "page" && value && keys.length === 1 && keys[0] === "graph")
    return { graph: uuid(params.get("graph") ?? ""), page: value };
  throw new Error("Unsupported Tine link: expected a graph, page or block");
}
