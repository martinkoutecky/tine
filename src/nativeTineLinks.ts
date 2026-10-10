/** Native URL transport over the ordinary leased backend command/event door.
 * Costs and failures are those of deep_links.rs. Browser/export backends omit
 * this surface, so they cannot manufacture graph identities. The share inbox
 * (ADR 0073, share_inbox.rs) rides on the same door: it exists only where the
 * native app does, and its items reach the graph only through
 * src/shareIngest.ts. */
import type { TineLink } from "./deepLinks";
import type { LinkTarget, LinkDelivery } from "./deepLinkNavigation";
export interface ShareInboxResource { path: string; name?: string | null; type?: string | null }
/** The ingest's durable record of an item's shaping (share_inbox.rs `Prepared`). */
export interface SharePrepared { markdown: string; day: string; baseline: number | null }
export interface ShareInboxItem {
  id: string;
  created?: number | null;
  text?: string | null;
  title?: string | null;
  url?: string | null;
  resources: ShareInboxResource[];
  prepared?: SharePrepared | null;
}
export interface NativeShareInbox {
  list(): Promise<{ items: ShareInboxItem[]; rejected: number }>;
  prepare(id: string, prepared: SharePrepared): Promise<void>;
  commit(id: string): Promise<void>;
  /** The native producers' "an item arrived" signal (mobile only). */
  subscribe(cb: () => void): Promise<() => void>;
}
export interface NativeTineLinks {
  identity(path?: string): Promise<string>;
  scanKnownGraphs(request: TineLink): Promise<LinkTarget[]>;
  take(): Promise<LinkDelivery[]>;
  handoff(target: LinkTarget): Promise<boolean>;
  subscribe(cb: () => void): Promise<() => void>;
  inbox?: NativeShareInbox;
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
    inbox: {
      list: () => call("share_inbox_list"),
      prepare: (id, prepared) => call("share_inbox_prepare", { id, prepared }),
      commit: (id) => call("share_inbox_commit", { id }),
      subscribe: async (cb) => {
        const { addPluginListener } = await import("@tauri-apps/api/core");
        const listener = await addPluginListener("native-integrations", "inboxChanged", () => cb());
        return () => { void listener.unregister(); };
      },
    },
  };
}
