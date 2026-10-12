import type { JSX } from "solid-js";
import { backend } from "../backend";
import { reportLinkOpenFailure } from "../components/ExternalLink";
import { graphOwner, readOwned } from "../owned";
import { pushToast } from "../toasts";
import type { FileLinkTarget } from "./relativeFileLink";
import type { SpanDomAttrs } from "./spans";

/** A link to a file by plain path (GH #457), resolved against the graph
 *  folder as OG does (relativeFileLink.ts). Inside the graph it opens through
 *  the native opener, which re-checks containment after symlinks; a link that
 *  points outside the graph folder is shown as broken and never opened (a link
 *  in a note is untrusted content: imported or malformed Markdown may name any
 *  path). Cost: one OS handoff per click. */
export function RelativeFileLink(props: {
  dest: string;
  target: FileLinkTarget;
  spanAttrs?: SpanDomAttrs;
  children: JSX.Element;
}): JSX.Element {
  const outside = () => props.target.kind === "outside";
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
        const target = props.target;
        if (target.kind === "outside") {
          pushToast(`${props.dest} points outside the graph folder, so Tine does not open it.`, "error");
        } else {
          void readOwned(graphOwner(), backend().openGraphFile(target.path, backend().graphBindingGeneration()))
            .catch((error) => reportLinkOpenFailure(props.dest, error));
        }
      }}
    >
      {props.children}
    </a>
  );
}
