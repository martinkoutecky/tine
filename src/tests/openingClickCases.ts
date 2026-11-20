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

/** The review's opening shapes (review 0e6bd6530 F1), `{P}` a task word and/or
 *  priority, with the groups the untitled parse finds (Markdown, Org; line ends
 *  as `\n`). Pinned beside the parser's own table
 *  (`page_preamble::opening_groups_and_pieces_share_one_untitled_parse`). */
const OPENING_SHAPES: readonly (readonly [shape: string, md: readonly string[], org: readonly string[]])[] = [
  ["{P}x\nk:: v", ["k:: v"], []],
  ["{P}k:: **v**", [], []],
  ["{P}#+k: v", [], []],
  ["{P}> quote\n#+k: v", ["#+k: v"], ["#+k: v"]],
  ["{P}> quote\nk:: v", ["k:: v"], []],
  ["{P}\n```\nc\n```\n#+k: v", ["#+k: v"], ["#+k: v"]],
  ["{P}\n~~~\nc\n~~~\nk:: v", ["k:: v"], []],
  ["{P}\n#+BEGIN_SRC\nc\n#+END_SRC\n#+k: v", ["#+k: v"], ["#+k: v"]],
  ["{P}\n#+BEGIN_NOTE\nn\n#+END_NOTE\nk:: v", ["k:: v"], []],
  ["{P}\n#+BEGIN_QUOTE\nq\n#+END_QUOTE\n#+k: v", ["#+k: v"], ["#+k: v"]],
  ["{P}\n#+BEGIN_QUERY\n(task TODO)\n#+END_QUERY\n#+k: v", ["#+k: v"], ["#+k: v"]],
  ["{P}\n{{query (task TODO)}}\nk:: v", ["k:: v"], []],
  ["{P}\n:LOGBOOK:\nCLOCK: x\n:END:\nk:: v", ["k:: v"], []],
  ["{P}\n:custom:\nv\n:END:\n#+k: v", ["#+k: v"], ["#+k: v"]],
  ["{P}\n:PROPERTIES:\n:a: 1\n:END:\nk:: v", [":PROPERTIES:\n:a: 1\n:END:\nk:: v"], [":PROPERTIES:\n:a: 1\n:END:"]],
];
const OPENING_PREFIXES = ["TODO ", "[#A] ", "TODO [#A] ", "TODO\t"] as const;
const OPENING_VARIANTS = ["lf", "crlf", "blank"] as const;

export type OpeningShapeCase = readonly [text: string, format: "md" | "org", groups: readonly string[], prefix: string];

function shapeCase(shape: string, prefix: string, variant: string, format: "md" | "org", groups: readonly string[]): OpeningShapeCase {
  const base = shape.replace("{P}", prefix);
  const text = variant === "crlf" ? base.replace(/\n/g, "\r\n") : variant === "blank" ? `\n${base}` : base;
  return [text, format, groups, prefix];
}

/** Every shape × task word/priority × LF, CRLF, blank-led × format. */
export const OPENING_SHAPE_CASES: readonly OpeningShapeCase[] = OPENING_SHAPES.flatMap(([shape, md, org]) =>
  OPENING_PREFIXES.flatMap((prefix) => OPENING_VARIANTS.flatMap((variant) =>
    (["md", "org"] as const).map((format) => shapeCase(shape, prefix, variant, format, format === "md" ? md : org)))));

/** Every shape × format once, the prefix and line-end variant rotating: the
 *  rows the page, Linked References and query List surfaces run. A last
 *  prose line keeps an Org page's final `#+` line opening text: at a
 *  pre-block's end it is the page header (`preamble_content_range`). */
export const OPENING_SHAPE_SURFACE_CASES: readonly OpeningShapeCase[] = OPENING_SHAPES.flatMap(([shape, md, org], i) =>
  (["md", "org"] as const).map((format, j) => shapeCase(`${shape}\nend`, OPENING_PREFIXES[(i + j) % OPENING_PREFIXES.length],
    OPENING_VARIANTS[(i + 2 * j) % OPENING_VARIANTS.length], format, format === "md" ? md : org)));

/** What the parser-and-render table asserts of one rendered opening row: the
 *  groups shown as written, the task word or priority shown as text with no
 *  task chrome, and `**v**` on a task line read as prose. */
export function expectOpeningShape(row: Element, [text, , groups, prefix]: OpeningShapeCase, expect: (v: unknown) => { toEqual(v: unknown): void; toBe(v: unknown): void; toContain(v: string): void }): void {
  expect([...row.querySelectorAll(".opening-group")].map((g) => g.textContent)).toEqual(groups);
  expect(row.querySelector(".block-marker, .block-priority, input[type=checkbox]")).toBe(null);
  expect(row.textContent ?? "").toContain(prefix.trim());
  if (text.includes("k:: **v**")) expect(row.querySelector(".opening-text strong")?.textContent).toBe("v");
}
