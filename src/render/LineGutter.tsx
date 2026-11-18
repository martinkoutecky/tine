import { newResizeObserver } from "../windowRealm";
import { For, Show, type JSX } from "solid-js";

/** Code card inset shared by reading and editing. O(1) from the displayed line count. */
export function codeGutterStyle(lineCount: number): JSX.CSSProperties {
  return { "--code-line-digits": String(String(lineCount).length) };
}

/** Calculator/code line gutter. Code mirrors line heights, including soft wraps,
 * without measuring layout; numbers stay on the first visual row of each line.
 * A card's gutter takes its text layout from the card's code element itself
 * (`followCodeText`), so no stylesheet can wrap one and not the other (GH #970).
 * O(lines of the displayed payload); callers supply already-parsed content.
 */
export function LineGutter(props: { lines: string[]; code?: boolean; ref?: (el: HTMLDivElement) => void }): JSX.Element {
  return <div ref={props.ref} class="calc-gutter" classList={{ "code-gutter": props.code }} aria-hidden="true">
    <For each={props.lines}>{(line, i) => <div class="gutter-row">
      <span class="calc-lineno">{i() + 1}</span>
      <Show when={props.code}><span class="gutter-mirror">{line || "\u00a0"}</span></Show>
    </div>}</For>
  </div>;
}

/** The inherited text properties that decide where a line of code breaks and how tall its rows are. */
const CODE_TEXT_LAYOUT = ["white-space", "overflow-wrap", "word-break", "line-break", "hyphens", "tab-size",
  "font-family", "font-size", "font-weight", "font-style", "font-stretch", "line-height", "letter-spacing", "word-spacing"];

/** GH #970: the card's code is the one answer to how its text lays out. Whatever rule styles it (Tine's wrap
 * setting, a theme, custom.css, any selector), the gutter copies the code's computed text layout, so the hidden
 * mirror lines break where the code's lines do. Re-read whenever the code's box changes size, which every
 * wrapping change does. O(1) per change; returns the disconnect. */
export function followCodeText(code: HTMLElement, gutter: HTMLElement): () => void {
  const copy = () => {
    const style = getComputedStyle(code);
    for (const name of CODE_TEXT_LAYOUT) gutter.style.setProperty(name, style.getPropertyValue(name));
  };
  copy();
  const observer = newResizeObserver(code, copy);
  observer?.observe(code);
  return () => observer?.disconnect();
}
