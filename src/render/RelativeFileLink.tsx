import { createMemo, type JSX } from "solid-js";
import { backend } from "../backend";
import { reportLinkOpenFailure } from "../components/ExternalLink";
import { node as docNode, pageByName } from "../document";
import { graphOwner, readOwned } from "../owned";
import { pushToast } from "../toasts";
import { resolvePageRelative } from "./relativeFileLink";
import type { SpanDomAttrs } from "./spans";

/** A relative file link (GH #457), resolved against the file of the page the
 *  block lives on. Inside the graph it opens through the native opener, which
 *  re-checks containment after symlinks; a link that climbs out of the graph
 *  folder is shown as broken and never opened (a link in a note is untrusted
 *  content: imported or malformed Markdown may name any path). Cost: O(path
 *  segments) per render; one OS handoff per click. */
export function RelativeFileLink(props: {
  dest: string;
  relative: string;
  blockId?: string;
  spanAttrs?: SpanDomAttrs;
  children: JSX.Element;
}): JSX.Element {
  const target = createMemo(() => {
    const pageName = props.blockId ? docNode(props.blockId)?.page : undefined;
    const pageFile = pageName ? pageByName(pageName)?.id : undefined;
    return pageFile ? resolvePageRelative(pageFile, props.relative) : null;
  });
  const outside = () => target()?.kind === "outside";
  return (
    <a
      class="external-link file-link"
      href={props.dest}
      data-broken-file-link={outside() ? "" : undefined}
      title={outside() ? "This link points outside the graph folder, so Tine does not open it" : undefined}
      {...(props.spanAttrs ?? {})}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        const resolved = target();
        if (!resolved) {
          pushToast(`Couldn't open ${props.dest}: its page has no file yet to resolve the link against.`, "error");
        } else if (resolved.kind === "outside") {
          pushToast(`${props.dest} points outside the graph folder, so Tine does not open it.`, "error");
        } else {
          void readOwned(graphOwner(), backend().openGraphFile(resolved.path, backend().graphBindingGeneration()))
            .catch((error) => reportLinkOpenFailure(props.dest, error));
        }
      }}
    >
      {props.children}
    </a>
  );
}
