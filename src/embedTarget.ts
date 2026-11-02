// The target of `{{embed ...}}`: the one TS answer, shared by the live macro and
// export. It takes what OG's `macro-embed-cp` (starts `[[`, ends `]]`, greedy)
// and the native static render (`render_embed`) take, so a page name may hold
// `]` or a nested `[[b]]`. Cost O(argument).

const inner = (arg: string, open: string, close: string): string | null => {
  const text = arg.trim();
  return text.length > open.length + close.length && text.startsWith(open) && text.endsWith(close)
    ? text.slice(open.length, -close.length)
    : null;
};

/** The page of `[[name]]`, or null. */
export const embedPageTarget = (arg: string): string | null => inner(arg, "[[", "]]");
/** The block of `((uuid))`, or null. */
export const embedBlockTarget = (arg: string): string | null => inner(arg, "((", "))");
