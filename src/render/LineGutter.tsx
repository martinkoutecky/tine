import { For, Show, type JSX } from "solid-js";

/** Calculator/code line gutter. Code mirrors line heights, including soft wraps,
 * without measuring layout; numbers stay on the first visual row of each line.
 * O(lines of the displayed payload); callers supply already-parsed content.
 */
export function LineGutter(props: { lines: string[]; code?: boolean }): JSX.Element {
  return <div class="calc-gutter" classList={{ "code-gutter": props.code }} aria-hidden="true">
    <For each={props.lines}>{(line, i) => <div class="gutter-row">
      <span class="calc-lineno">{i() + 1}</span>
      <Show when={props.code}><span class="gutter-mirror">{line || "\u00a0"}</span></Show>
    </div>}</For>
  </div>;
}
