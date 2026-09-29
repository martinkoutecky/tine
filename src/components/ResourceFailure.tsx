import { Show, type JSX } from "solid-js";
import type { Resource } from "solid-js";

/**
 * The failure row for a panel whose EMPTINESS WOULD LIE (master c5279d186).
 *
 * `readOr` stops a rejected resource throwing into render, which is right for a
 * decorative value: a missing icon degrades to what the site already drew for
 * "not loaded yet". But a references panel or a page list that renders empty is
 * asserting "there are none", and that is false. Those panels pair `readOr` with
 * this row, so the user can tell "nothing here" from "we could not find out".
 *
 * Deliberately not a boundary: a boundary sized to one panel costs its subtree
 * and state, while a panel that knows its own resource failed keeps its header,
 * controls and place. `FailureBoundary` stays for what a component cannot
 * anticipate. The failure's message is never shown (it can carry the user's own
 * content); it goes to the opt-in debug log via `readOr`.
 */
export function ResourceFailure(props: {
  /** Any resource this panel needs; the row shows when one has failed. */
  of: Resource<unknown> | readonly Resource<unknown>[];
  /** What could not be loaded, in the user's words: "references", "this list". */
  what: string;
  /** Refetch, when the panel has one. Omitted for a resource that cannot retry. */
  onRetry?: () => void;
}): JSX.Element {
  const failed = () => {
    const list = Array.isArray(props.of) ? props.of : [props.of as Resource<unknown>];
    return list.some((resource) => resource.error !== undefined);
  };
  return (
    <Show when={failed()}>
      <div class="resource-failure" role="alert">
        <span class="resource-failure-text">Couldn’t load {props.what}.</span>
        <Show when={props.onRetry}>
          <button type="button" class="resource-failure-retry" onClick={() => props.onRetry?.()}>
            Retry
          </button>
        </Show>
      </div>
    </Show>
  );
}
