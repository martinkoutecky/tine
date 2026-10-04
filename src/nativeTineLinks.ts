/** Native URL transport over the ordinary leased backend command/event door.
 * Costs and failures are those of deep_links.rs. Browser/export backends omit
 * this surface, so they cannot manufacture graph identities. */
import type { TineLink } from "./deepLinks";
import type { LinkTarget, LinkDelivery } from "./deepLinkNavigation";
export interface NativeTineLinks {
  identity(path?: string): Promise<string>;
  scanKnownGraphs(request: TineLink): Promise<LinkTarget[]>;
  take(): Promise<LinkDelivery[]>;
  handoff(target: LinkTarget): Promise<boolean>;
  subscribe(cb: () => void): Promise<() => void>;
}
export function nativeTineLinks(
  call: <T>(command: string, args?: Record<string, unknown>) => Promise<T>,
  subscribe: NativeTineLinks["subscribe"],
): NativeTineLinks {
  return {
    identity: (path) => call("graph_link_identity", { path }),
    scanKnownGraphs: (request) => call("scan_known_graphs_for_link", { request }),
    take: () => call("take_tine_links"),
    handoff: (target) => call("handoff_tine_link", { target }),
    subscribe,
  };
}
