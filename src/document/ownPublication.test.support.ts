import { applyGraphChangesBulk } from "./external";
import { pageIdentityKey } from "../pageIdentity";

let revision = 0;
/** Deliver the existing native own-publication envelope for one changed page. */
export function publishOwnPage(name: string): Promise<void> {
  const rev = String(++revision);
  return applyGraphChangesBulk({ changes: [], answers: { rev, inventoryChanged: false, blockRefCounts: {},
    summary: { rev, pages: [pageIdentityKey(name)], blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } } });
}
