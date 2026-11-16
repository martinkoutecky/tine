// The code-fence opener forms of the language-label tests, shared by the pure table in
// `codeFence.test.ts` (every form, whole or mixed block, both formats) and the routed
// `components/Page.codeLanguage.test.tsx` (which mounts every form once per engine and
// format, and every entry method over the three REPRESENTATIVE forms).
// lsdoc v0.5.11 parse.rs/org.rs: fence_marker/fence_lang and block_begin/begin_lang.
export type Format = "md" | "org";
export interface Opener { open: string; close: string; lang: string }

export const FORMATS: readonly Format[] = ["md", "org"];

export const OPENERS: readonly Opener[] = [
  ["```", "```", ""], ["~~~", "~~~", ""],
  ["```js :results output", "```", "js"], ["~~~\t js :results output", "~~~", "js"],
  ["````js :results output", "````", "`js"], ["~~~~~", "~~~~~", "~~"],
  ["  ```", "  ```", ""], ["  #+BEGIN_SRC", "  #+END_SRC", ""],
  ["#+BEGIN_SRC", "#+END_SRC", ""], ["#+begin_src", "#+end_src", ""],
  ["#+BeGiN_sRc", "#+EnD_sRc", ""],
  ["#+BEGIN_SRC js :results output", "#+END_SRC", "js"],
  ["#+begin_src\tjs :results output", "#+end_src", "js"],
  ["#+BEGIN_SRC ", "#+END_SRC", ""],
  ["#+BEGIN_EXAMPLE", "#+END_EXAMPLE", ""], ["#+begin_example", "#+end_example", ""],
].map(([open, close, lang]) => ({ open, close, lang }));

/** The forms whose routed cases take every entry method: Markdown backticks, the unlabelled
 *  lower-case `#+begin_src` (the form that needs a separator, where paste once failed) and
 *  `#+begin_example` (the other separator-bearing literal). */
export const REPRESENTATIVE = ["```", "#+begin_src", "#+begin_example"];

export const ENTRIES = ["leave", "typing", "paste", "programmatic paste", "IME", "drop", "replacement", "plain input"] as const;
export type Entry = typeof ENTRIES[number];

export function codeRaw({ open, close }: Opener, mixed: boolean): string {
  return `${mixed ? "前 🐈\n" : ""}${open}\nx\n${close}${mixed ? "\nAfter" : ""}`;
}

/** The block kind the opener is read as, and whether typing a language into it needs a separator. */
export const literalOf = (open: string): "example" | "src" => /begin_example/i.test(open) ? "example" : "src";
export const needsSeparator = (open: string): boolean => ["#+begin_src", "#+begin_example"].includes(open.trimStart().toLowerCase());
