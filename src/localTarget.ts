// The TypeScript door to the one owner of "which local file or attachment does
// this authored destination name" (crates/tine-core/src/local_target.rs). The
// answer is computed once, in Rust, from the parser's own node; this module is
// the only place that turns it into the branded `AssetName`.
import { local_target_json } from "./render/wasm/lsdoc_wasm.js";
import type { Url } from "./render/ast";

declare const assetNameBrand: unique symbol;
/** A name under the graph's `assets/` folder ("" is the folder itself). Only
 * `localTarget` makes one; a cast elsewhere defeats the single owner. */
export type AssetName = string & { readonly [assetNameBrand]: true };

/** Where the destination was written. */
export type LocalTargetOrigin =
  | "md-link" | "md-embed" | "org-link" | "org-bare" | "img-macro" | "html-resource" | "page-icon";

export type LocalRoute =
  | { kind: "page_ref"; name: string }
  | { kind: "block_ref"; id: string }
  | { kind: "web" }
  | { kind: "mail" }
  | { kind: "app_scheme"; scheme: string }
  | { kind: "refused" }
  | { kind: "fragment"; href: string }
  | { kind: "attachment"; name: AssetName }
  | { kind: "graph_file"; path: string }
  | { kind: "outside_file"; path: string; reason: "absolute" | "escapes" }
  | { kind: "drive"; path: string }
  | { kind: "file_url"; url: string }
  | { kind: "page_relative"; path: string }
  | { kind: "refused_malformed" };

export type LocalPdf =
  | { kind: "asset_reader"; name: AssetName }
  | { kind: "outside_reader"; fallback: AssetName | null };

export interface LocalTarget {
  route: LocalRoute;
  pdf: LocalPdf | null;
}

/** The parser's link node's `url`, or the text of a macro argument, raw HTML
 * attribute or page icon value. */
export type LocalDestination = { url: Url } | { text: string };

// The door's numbering (crates/lsdoc-wasm `local_target_json`): a plain text
// destination is 0, a Url tag its index here; an origin its index in ORIGINS.
// An unknown name is -1, which the door answers with `null`.
const URL_TYPES: readonly string[] = ["", "page_ref", "block_ref", "search", "file", "embed_data", "complex"];
const ORIGINS: readonly LocalTargetOrigin[] = [
  "md-link", "md-embed", "org-link", "org-bare", "img-macro", "html-resource", "page-icon",
];

/** Ask the owner. Pure, O(destination bytes), no I/O. Throws on an input the
 * owner refuses to read (an unknown origin), which is a programming error. */
export function localTarget(destination: LocalDestination, origin: LocalTargetOrigin): LocalTarget {
  const at = ORIGINS.indexOf(origin);
  const json = "text" in destination
    ? local_target_json(0, destination.text, undefined, undefined, at)
    : destination.url.type === "complex"
      ? local_target_json(URL_TYPES.indexOf("complex"), "", destination.url.protocol, destination.url.link, at)
      : local_target_json(URL_TYPES.indexOf(destination.url.type), destination.url.v, undefined, undefined, at);
  const answer = JSON.parse(json) as LocalTarget | null;
  if (answer === null) throw new Error(`local target: unreadable destination for ${origin}`);
  return answer;
}
