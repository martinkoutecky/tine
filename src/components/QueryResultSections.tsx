/** Friendly-search result shell. Answers which family is pending, failed,
 * empty, or truncated; work is O(1) per family plus rendering supplied rows.
 * A failed read shows no rows. Callers need only the execution's two hit sets
 * and its per-family truncation flags. */
import { For, Show, createUniqueId, type JSX } from "solid-js";

export interface ResultFamily {
  kind: "page" | "block";
  /** Remains available when this family has no rows. */
  control?: JSX.Element;
  hits: number;
  hasMore: boolean;
  body: JSX.Element;
}

/** Render both families even at zero rows. `failure` suppresses bodies and
 * empties; `pending` suppresses old rows until the current read completes. */
export function QueryResultSections(props: {
  families: ResultFamily[];
  pending: boolean;
  failure: string | null;
}): JSX.Element {
  const mount = createUniqueId();
  return <div class="query-result-sections">
    <For each={props.families}>{(family) => <section
      class="query-result-section"
      data-query-result-kind={family.kind}
      aria-labelledby={`query-results-${mount}-${family.kind}`}
      aria-busy={props.pending ? "true" : "false"}
    >
      <header class="query-result-section-header"><h3 id={`query-results-${mount}-${family.kind}`}>
        {family.kind === "page" ? "Pages" : "Blocks"} <span class="query-result-section-count">{family.hits}</span>
      </h3>{family.control}</header>
      <Show when={props.failure}><p role="alert">{props.failure}</p></Show>
      <Show when={!props.failure && props.pending}><p role="status">Searching…</p></Show>
      <Show when={!props.failure && !props.pending}>
        <Show when={family.hits > 0} fallback={<p>{family.kind === "page" ? "No matching pages." : "No matching blocks."}</p>}>
          {family.body}
        </Show>
      </Show>
      <Show when={!props.pending && !props.failure && family.hasMore}>
        <p>More {family.kind === "page" ? "pages" : "blocks"} match than are shown.</p>
      </Show>
    </section>}</For>
  </div>;
}
