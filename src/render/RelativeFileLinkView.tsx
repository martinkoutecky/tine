import type { JSX } from "solid-js";
import { backend } from "../backend";
import { openOutboundLink, outboundAnchorAttrs, reportLinkOpenFailure } from "../components/ExternalLink";
import type { AnchorDecision } from "../linkSchemePolicy";
import { bindingOwner } from "../owned";
import { withFileConsent } from "../fileConsent";
import { graphMeta } from "../graphSession";
import { filePathUrl, resolvedFilePath, type FileLinkTarget } from "./relativeFileLink";
import type { SpanDomAttrs } from "./spans";

/** A graph-authored file path (GH #457). In-graph links keep the guarded graph
 * opener; explicitly authored outside paths use the existing file: opener.
 * Both ask first when the target cannot be proven inside the graph (tine#985).
 * Cost: lexical path resolution on render, one native command per click. */
export function RelativeFileLink(props: {
  dest: string;
  decision: AnchorDecision;
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
      // tine#985: a path that leaves the graph and may come back asks first,
      // through the same consent as every other file: link; both calls carry
      // this click's path and binding generation.
      const api = backend(), generation = api.graphBindingGeneration(), path = target.path;
      void withFileConsent(bindingOwner(), purpose => api.openGraphFile(path, generation, purpose))
        .catch((error) => reportLinkOpenFailure(props.dest, error));
    }
  };
  return (
    <a
      class="external-link file-link"
      title={resolved()}
      {...(props.spanAttrs ?? {})}
      {...outboundAnchorAttrs(props.dest, props.decision, activate)}
    >
      {props.children}
    </a>
  );
}
