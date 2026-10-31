import type { JSX } from "solid-js";
import { FailureBoundary } from "./FailureBoundary";

/** One error owner per displayed block. Recovery allocates only on failure;
 * healthy blocks add no DOM, parsing, graph reads/writes, observers or IPC.
 * Live rows keep their editor outside this boundary; DTOs can open the source. */
export function BlockRenderBoundary(props: { raw: string; onOpen?: () => void; children: JSX.Element }): JSX.Element {
  return <FailureBoundary region="This block" recovery={() =>
    <pre class="block-content block-render-raw" style={{ "white-space": "pre-wrap", "overflow-wrap": "anywhere" }}
      tabIndex={props.onOpen ? 0 : undefined} onClick={props.onOpen}
      onKeyDown={props.onOpen ? event => {
        if (event.key === "Enter" || event.key === " ") { event.preventDefault(); props.onOpen?.(); }
      } : undefined}>{props.raw}</pre>
  }>{props.children}</FailureBoundary>;
}
