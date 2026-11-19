import type { Url } from "./ast";
import type { Format } from "../types";

/** Browser link destination; O(destination length), with no AST conversion.
 * Empty protocols use the bare link. Native export retains its Option policy. */
export function urlDest(url: Url, format?: Format): string {
  switch (url.type) {
    case "page_ref":
    case "block_ref":
    case "search":
    case "file":
    case "embed_data":
      return url.v;
    case "complex":
      if (!url.protocol || url.link == null) return url.link ?? "";
      // Org also splits `C:path` into protocol and link. Inserting `//` here
      // would make a drive-relative path appear absolute to the link policy.
      return `${url.protocol}${format === "org" && /^[a-z]$/i.test(url.protocol) ? ":" : "://"}${url.link}`;
  }
}

