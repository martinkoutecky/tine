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

/** Opening text with a task word or priority on its own line, then a drawer,
 *  then more text (review e852e1b58 F2): opening text has no block header, so
 *  each shows as the two lines written, the drawer hidden as in every body.
 *  Org and Markdown; LOGBOOK and a custom drawer. */
export const OPENING_DRAWER_CASES: readonly (readonly [text: string, format: "md" | "org", lines: readonly string[]])[] =
  (["md", "org"] as const).flatMap((format) => [
    "TODO \n:LOGBOOK:\nCLOCK: [2026-11-19 Thu 10:00]\n:END:\nNext",
    "[#A] \n:LOGBOOK:\nCLOCK: [2026-11-19 Thu 10:00]\n:END:\nNext",
    "TODO [#A] \n:LOGBOOK:\nCLOCK: [2026-11-19 Thu 10:00]\n:END:\nNext",
    "TODO \n:custom:\nv\n:END:\nNext",
    "[#B]\n:custom:\nv\n:END:\nNext",
    "DONE [#C]\t\n:custom:\nv\n:END:\nNext",
  ].map((text) => [text, format, [text.split("\n")[0], "Next"]] as const));

/** The lines the opening text inside `el` shows, piece by piece: a blank
 *  piece is one empty line, and a `<br>` starts a new line. */
export function shownLines(el: Element): string[] {
  return [...el.querySelectorAll(".opening-group, .opening-text, .opening-blank")].flatMap((piece) => {
    if (piece.classList.contains("opening-blank")) return [""];
    const out = [""];
    const walk = (node: Node) => {
      if (node.nodeName === "BR") out.push("");
      else if (node.nodeType === Node.TEXT_NODE) out[out.length - 1] += node.textContent ?? "";
      else node.childNodes.forEach(walk);
    };
    walk(piece);
    return out.flatMap((line) => line.split("\n"));
  });
}
