// The ONE formatter for inserting a tag token (`#name` vs `#[[multi word]]`).
// Bare `#name` is emitted only when lsdoc's tag lexer (lsdoc inline.rs,
// `parse_tag_name`) would consume the whole name and stop at the boundary:
// no whitespace, none of the hard stops (`# , ! ? ' " :`), no `[`/`]`
// (page-ref brackets), and no trailing `.`/`;` (stripped as a trailing
// delimiter run). Anything else — including non-ASCII letters, which are
// valid bare — stays unbracketed, matching what OG users type by hand.
const TAG_UNSAFE = /[\s#,!?'":[\]]/;
const TAG_TRAILING_DELIM = /[.;]$/;

export function tagRef(pageName: string): string {
  return !pageName || TAG_UNSAFE.test(pageName) || TAG_TRAILING_DELIM.test(pageName)
    ? `#[[${pageName}]]`
    : `#${pageName}`;
}
