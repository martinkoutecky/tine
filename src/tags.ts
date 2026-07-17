// The ONE formatter for inserting a tag token (`#name` vs `#[[multi word]]`).
// Bare `#name` is emitted only when lsdoc's tag lexer (lsdoc inline.rs,
// `parse_tag_name`) would consume the whole name and stop at the boundary:
// no whitespace, none of the hard stops (`# , ! ? ' " :`), no `[`/`]`
// (page-ref brackets), and no trailing `.`/`;` (stripped as a trailing
// delimiter run). Anything else — including non-ASCII letters, which are
// valid bare — stays unbracketed, matching what OG users type by hand.
const TAG_HARD_STOP = /[\s#,!?'":[\]]/;
const TAG_TRAILING_DELIM = /[.;]$/;

/** True while a user-entered bare-tag prefix can still become one lsdoc tag.
 * Empty is valid here because a lone `#` opens the tag picker. Trailing `.` and
 * `;` remain possible internal characters until the user finishes the name. */
export function isBareTagPrefix(value: string): boolean {
  return !TAG_HARD_STOP.test(value);
}

export function isBareTagName(pageName: string): boolean {
  return !!pageName && isBareTagPrefix(pageName) && !TAG_TRAILING_DELIM.test(pageName);
}

export function tagRef(pageName: string): string {
  return !isBareTagName(pageName)
    ? `#[[${pageName}]]`
    : `#${pageName}`;
}
