import type { Url } from "./ast";

/** Browser link destination; O(destination length), with no AST conversion.
 * Empty protocols use the bare link. Native export retains its Option policy. */
export function urlDest(url: Url): string {
  switch (url.type) {
    case "page_ref":
    case "block_ref":
    case "search":
    case "file":
    case "embed_data":
      return url.v;
    case "complex":
      return url.protocol && url.link != null ? `${url.protocol}://${url.link}` : url.link ?? "";
  }
}

