import { checkViewer } from "./check-quick-share-viewer.mjs";

/** Resolves only after the deployed, hash-verified viewer at tine.page has
 * rendered this writer's entire fixture corpus; throws otherwise. Every build
 * offers links (D-20261117-QSG), so a release refuses on a throw. No network writes. */
export async function checkQuickShareReadiness() {
  return checkViewer("https://tine.page/s/1/");
}
