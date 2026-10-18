// A quick share in the graph (D-20261017-QS3, D-20261018-Q1; docs/contracts/export-definitions.md): the share's
// export definition is a query block carrying `tine.share:: <login>/<gist_id>`. The key never enters the graph
// (it stays in this device's `quick-shares.json`), so the property names the gist and nothing that opens it.
// Every write here is an ordinary block edit through the document layer (setBlockProperty, appendQueryBlock) and
// its normal guarded save; this module adds no write path.
import { backend } from "../backend";
import { bindingCurrent, captureBinding } from "../binding";
import {
  appendQueryBlock, blockProperty, blockWritable, childIds, ensurePageLoaded, flushPage, node, pageByName, pageRoots,
  reportPageLoadRefusal, setBlockProperty,
} from "../document";
import { SHARE_PROPERTY, ensureExportsPage } from "../exportsPage";
import { graphOwner, readOwned } from "../owned";

/** The property value of a share: `<login>/<gist_id>`, never the key. */
export function shareValue(login: string, gistId: string): string {
  return `${login}/${gistId}`;
}

/** Where the share dialog started: the query block it opened on (when its query was not changed), and the query
 *  text a new definition carries. */
export interface ShareDefinitionTarget {
  hostBlockId: string | null;
  macro: string;
}

/** Record a created share in the graph. On the query block it started from, when that block can be edited and
 *  does not already name another share (re-sharing that one is phase 3; overwriting would drop the earlier share
 *  from the Exports page while its gist lives on); otherwise as a new definition on the exports page. Answers the
 *  page that now holds it, or throws with a reason a toast can show. */
export async function saveShareDefinition(target: ShareDefinitionTarget, value: string): Promise<string> {
  const binding = captureBinding();
  const host = target.hostBlockId;
  if (host && blockWritable(host)) {
    const current = blockProperty(host, SHARE_PROPERTY);
    if (current === null || current === value) {
      if (current === null) setBlockProperty(host, SHARE_PROPERTY, value);
      const page = node(host)?.page;
      if (blockProperty(host, SHARE_PROPERTY) !== value || !page) throw new Error("the query block can't be edited");
      if (!(await flushPage(page))) throw new Error(`couldn't save ${page}`);
      return page;
    }
  }
  const page = await ensureExportsPage();
  if (!bindingCurrent(binding)) throw new Error("graph-changed");
  if (!(await appendQueryBlock(page, target.macro, [[SHARE_PROPERTY, value]]))) throw new Error(`couldn't write to ${page}`);
  return page;
}

/** The blocks of a loaded page, depth first. */
function pageBlocks(page: string): string[] {
  const out: string[] = [];
  const walk = (ids: readonly string[]) => { for (const id of ids) { out.push(id); walk(childIds(id)); } };
  walk(pageRoots(page));
  return out;
}

/** Remove `tine.share:: <value>` from every block that carries it (a share was deleted or forgotten), wherever
 *  the definition lives. The engine's `(property tine.share)` finds the pages; each is loaded into the working set
 *  (a read; a loaded page keeps its unsaved edits) and the property goes through setBlockProperty and the normal
 *  save. Answers the pages it changed; throws when a page holding the share could not be changed, so the caller
 *  says so instead of leaving a stale listing unmentioned. */
export async function removeShareProperty(value: string): Promise<string[]> {
  const owner = graphOwner();
  const parsed = await readOwned(owner, backend().parseQuery(`(property ${SHARE_PROPERTY})`, "og"));
  if (parsed.kind === "stale") return [];
  const result = await readOwned(owner, backend().queryRun(parsed.value.query, parsed.value.view));
  if (result.kind === "stale" || result.value.anchor !== "block") return [];
  const pages = result.value.groups
    .filter((group) => group.blocks.some((block) => block.raw.includes(value)))
    .map((group) => ({ name: group.page, kind: group.kind }));
  const changed: string[] = [];
  const failed: string[] = [];
  for (const { name, kind } of pages) {
    if (!pageByName(name)) {
      const loaded = await readOwned(owner, backend().getPage(name, kind));
      if (loaded.kind === "stale") return changed;
      if (!loaded.value) continue;
      const refusal = ensurePageLoaded(loaded.value);
      if (refusal) { reportPageLoadRefusal(refusal, "Its share property was not removed."); failed.push(name); continue; }
    }
    const ids = pageBlocks(name).filter((id) => blockProperty(id, SHARE_PROPERTY) === value);
    if (!ids.length) continue;
    for (const id of ids) setBlockProperty(id, SHARE_PROPERTY, null);
    if (ids.some((id) => blockProperty(id, SHARE_PROPERTY) === value) || !(await flushPage(name))) failed.push(name);
    else changed.push(name);
  }
  if (failed.length) throw new Error(`couldn't remove ${SHARE_PROPERTY}:: ${value} from ${failed.join(", ")}`);
  if (result.value.exceeded) throw new Error(`more blocks carry ${SHARE_PROPERTY} than one query answers; check the Exports page`);
  return changed;
}
