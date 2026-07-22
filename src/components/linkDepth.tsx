import { createContext, type JSX } from "solid-js";

// OG Logseq's max-depth-of-links / :link-depth invariant. A top-level macro
// starts at zero; each live ref/query/embed surface increments the shared depth.
export const MAX_DEPTH_OF_LINKS = 5;
export const LinkDepthContext = createContext(0);

export function LinkDepthWarning(): JSX.Element {
  return <p class="warning text-sm link-depth-warning">Embed depth is too deep</p>;
}
