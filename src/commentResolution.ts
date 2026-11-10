import { node, formatForBlock } from "./document";
import { resolvedOf } from "./comments";
import { facetsOf } from "./render/facets";

/** Call with the outermost root: a reply's own handwritten flag has no effect. */
export function threadResolved(root: string): boolean {
  const n = node(root);
  return !!n && resolvedOf(facetsOf(n.raw, formatForBlock(root)).properties);
}
