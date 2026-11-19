/** Opening text whose rendered control owns its click (review fe3946dc9 F1):
 *  a click on the control (`target`) must not promote the opening text; the
 *  opening row's edit gesture is the ordinary block predicate's
 *  (`forbidsEditEntry`). Markdown and Org, inside quote and note too. */
export const OPENING_CONTROL_CASES: readonly (readonly [text: string, format: "md" | "org", target: string])[] = [
  ["Intro [label](https://example.com)", "md", "a"],
  ['Intro <a href="https://example.com">x</a>', "md", "a"],
  ['[:a {:href "https://example.com"} "x"]', "md", "a"],
  ["<details><summary>S</summary>body</details>", "md", "summary"],
  ['<video src="v.mp4" controls></video>', "md", "video"],
  ['<audio src="a.mp3" controls></audio>', "md", "audio"],
  ["> [label](https://example.com)", "md", "a"],
  ['#+BEGIN_NOTE\n<a href="https://example.com">x</a>\n#+END_NOTE', "md", "a"],
  ["Intro [[https://example.com][label]]", "org", "a"],
  ["#+BEGIN_QUOTE\n[[https://example.com][q]]\n#+END_QUOTE", "org", "a"],
  ["#+BEGIN_NOTE\n[[https://example.com][n]]\n#+END_NOTE", "org", "a"],
];

/** A click on `el` as a pointer delivers it: press, release, click. */
export function pointerClick(el: Element): void {
  for (const type of ["mousedown", "mouseup", "click"]) {
    el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
  }
}
