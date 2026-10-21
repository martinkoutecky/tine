/** Where a text offset lands after `before` was rewritten into `after` by a
 *  programmatic canonicalisation (a time-tracking clock-in appended to the
 *  block, GH #741) rather than by the user typing.
 *
 *  The rewrite is treated as one replaced span: the longest common prefix and
 *  suffix are unchanged, so an offset in the prefix stays, an offset in the
 *  suffix moves by the length change, and an offset inside the replaced span
 *  goes to the end of its replacement. Cost O(min(len)). */
export function offsetThroughRewrite(before: string, after: string, offset: number): number {
  if (before === after) return offset;
  const limit = Math.min(before.length, after.length);
  let prefix = 0;
  while (prefix < limit && before.charCodeAt(prefix) === after.charCodeAt(prefix)) prefix++;
  let suffix = 0;
  while (
    suffix < limit - prefix &&
    before.charCodeAt(before.length - 1 - suffix) === after.charCodeAt(after.length - 1 - suffix)
  ) suffix++;
  if (offset <= prefix) return offset;
  if (offset >= before.length - suffix) return offset + after.length - before.length;
  return after.length - suffix;
}
