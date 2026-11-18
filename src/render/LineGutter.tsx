import { For, Show, type JSX } from "solid-js";

/** Code card inset shared by reading and editing. O(1) from the displayed line count. */
export function codeGutterStyle(lineCount: number): JSX.CSSProperties {
  return { "--code-line-digits": String(String(lineCount).length) };
}

/** Calculator/code line gutter. Code mirrors line heights, including soft wraps,
 * without measuring layout; numbers stay on the first visual row of each line.
 * A code card renders its gutter inside its `code` element, so the mirror inherits
 * whatever text layout styles the code (Tine's wrap setting, a theme, custom.css,
 * live or not) and cannot break a line elsewhere (GH #970).
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
