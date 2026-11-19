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
      // The Org projection uses the same fields for C:path and C://path,
      // even when the drive letter is escaped. Recover the authored delimiter.
      if (format === "org" && (/^[a-z]$/i.test(url.protocol) || url.protocol.includes("%"))
          && !full?.startsWith(`[[${url.protocol}://`)) {
        return `${url.protocol}:${url.link}`;
      }
      return `${url.protocol}://${url.link}`;
  }
}

