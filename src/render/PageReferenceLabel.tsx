import { Show, type JSX } from "solid-js";
import { showBrackets } from "../ui";

/** The label of an unaliased page reference, including the graph's bracket
 * preference. O(1) per visible link; does not resolve or load the destination. */
export function PageReferenceLabel(props: { children: JSX.Element }): JSX.Element {
  return <><Show when={showBrackets()}><span class="bracket">[[</span></Show>{props.children}<Show when={showBrackets()}><span class="bracket">]]</span></Show></>;
}
