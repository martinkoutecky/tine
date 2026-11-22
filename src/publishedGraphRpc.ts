import type { QuickShareReason } from "./quickShare/quickShare";
import type { BrowserGraph } from "./render/wasm-core/lsdoc_wasm";

export type GraphMethod = Extract<keyof BrowserGraph, `${string}_json`>;
export type GraphCall = {
  [Method in GraphMethod]: [Method, ...Parameters<BrowserGraph[Method]>];
}[GraphMethod];
export type GraphRead = (call: GraphCall, queryLocation?: string) => Promise<string>;

export type GraphRequest = { id: number } & (
  | { method: "open"; args: [ArrayBuffer, boolean, string | undefined] }
  | { method: "ping"; args: [] }
  | { method: GraphMethod; args: Parameters<BrowserGraph[GraphMethod]>[number][]; queryLocation?: string }
);

export type GraphReply = { id: number } & (
  | { json: string }
  | { error: string; fatal?: true; shareReason?: QuickShareReason; queryLimit?: true }
);

export class PublishedQueryLimitError extends Error {}
