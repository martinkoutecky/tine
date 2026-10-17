import { documentOf } from "./windowRealm";
import { clearOnBindingInvalidated } from "./binding";

export interface YoutubePlayer {
  seekTo(seconds: number, allowSeekAhead: boolean): void;
  getCurrentTime(): number;
  destroy?(): void;
}

const youtubePlayers = new Map<string, YoutubePlayer>();
const youtubePlayed = new Map<string, number>();
let youtubePlayOrder = 0;
clearOnBindingInvalidated(() => {
  youtubePlayers.clear();
  youtubePlayed.clear();
  youtubePlayOrder = 0;
});

export function registerYoutubePlayer(id: string, player: YoutubePlayer): void {
  youtubePlayers.set(id, player);
}

export function recordYoutubePlayback(id: string): void {
  youtubePlayed.set(id, ++youtubePlayOrder);
}

export function unregisterYoutubePlayer(id: string, player: YoutubePlayer | undefined): void {
  if (youtubePlayers.get(id) === player) youtubePlayers.delete(id);
  youtubePlayed.delete(id);
}

// Intentional OG divergence: youtube.cljs/get-player uses only the last iframe
// preceding the target. A timestamp contains seconds, not a video id: infer its
// video from that preceding embed in its own pane/sidebar item, then prefer the
// most recently played copy of that video anywhere in the same document. With
// no association, use playback history, then OG's original DOM-order fallback.
// Both timestamp clicks and timestamp insertion use this single selection boundary.
export function youtubePlayerForTarget(target: Node): YoutubePlayer | undefined {
  if (typeof document === "undefined" || typeof Node === "undefined") return undefined;
  const iframes = Array.from(documentOf(target).getElementsByTagName("iframe"))
    .filter((node) => node.src.includes("youtube.com"))
    .filter((node) => node.isConnected);
  const preceding = iframes.filter((node) => (node.compareDocumentPosition(target) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0);
  const surface = (node: Node) => (node.nodeType === 1 ? node as Element : node.parentElement)?.closest(".rs-item, [data-pane-id]");
  const associated = preceding.filter((node) => surface(node) === surface(target)).at(-1);
  const videoId = (node: HTMLIFrameElement) => /\/embed\/([\w-]{11})(?:[?/]|$)/.exec(node.src)?.[1];
  const id = associated && videoId(associated);
  const available = iframes.filter((node) => youtubePlayers.has(node.id));
  const recentFirst = (nodes: HTMLIFrameElement[]) => nodes.sort((a, b) => (youtubePlayed.get(b.id) ?? 0) - (youtubePlayed.get(a.id) ?? 0));
  const matching = id ? recentFirst(available.filter((node) => videoId(node) === id)) : [];
  const chosen = matching.find((node) => youtubePlayed.has(node.id))
    ?? matching.find((node) => node === associated)
    ?? matching[0]
    ?? recentFirst(available.filter((node) => youtubePlayed.has(node.id)))[0]
    ?? preceding.at(-1);
  return chosen ? youtubePlayers.get(chosen.id) : undefined;
}

