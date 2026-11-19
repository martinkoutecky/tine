import type { JSX } from "solid-js";
import { backend } from "../backend";
import { openOutboundLink, outboundAnchorAttrs, reportLinkOpenFailure } from "../components/ExternalLink";
import { graphOwner, readOwned } from "../owned";
import { graphMeta } from "../graphSession";
import { filePathUrl, resolvedFilePath, type FileLinkTarget } from "./relativeFileLink";
import type { SpanDomAttrs } from "./spans";

/** A graph-authored file path (GH #457). In-graph links keep the guarded graph
 * opener; explicitly authored outside paths use the existing file: opener.
 * Cost: lexical path resolution on render, one native command per click. */
export function RelativeFileLink(props: {
  dest: string;
  target: FileLinkTarget;
  spanAttrs?: SpanDomAttrs;
  children: JSX.Element;
}): JSX.Element {
  const resolved = () => resolvedFilePath(props.target, graphMeta()?.root ?? "");
  const activate = () => {
    const target = props.target;
    if (target.kind === "outside") {
      void openOutboundLink(filePathUrl(resolved()));
    } else {
      void readOwned(graphOwner(), backend().openGraphFile(target.path, backend().graphBindingGeneration()))
        .catch((error) => reportLinkOpenFailure(props.dest, error));
    }
  };
  return (
    <a
      class="external-link file-link"
      {...outboundAnchorAttrs(props.dest, activate)}
      title={resolved()}
      {...(props.spanAttrs ?? {})}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        activate();
      }}
    >
      {props.children}
    </a>
  );
}
