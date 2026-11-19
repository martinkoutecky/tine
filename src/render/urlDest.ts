import type { Url } from "./ast";
import type { Format } from "../types";

/** Browser link destination; O(destination length), with no AST conversion.
 * Empty protocols use the bare link. Native export retains its Option policy. */
export function urlDest(url: Url, format?: Format, full?: string): string {
  switch (url.type) {
    case "page_ref":
    case "block_ref":
    case "search":
    case "file":
    case "embed_data":
      return url.v;
    case "complex":
      if (!url.protocol || url.link == null) return url.link ?? "";
      // The Org projection uses the same fields for C:path and C://path.
      // Its full text preserves the delimiter at the start of the destination.
      if (format === "org" && /^[a-z]$/i.test(url.protocol) && !full?.startsWith(`[[${url.protocol}://`)) {
        return `${url.protocol}:${url.link}`;
      }
      return `${url.protocol}://${url.link}`;
  }
}

