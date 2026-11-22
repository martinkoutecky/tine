import { createContext } from "solid-js";

/** Source identity for queries rendered from a page opening, property, or
 * user macro expansion. Nested renderers append the parser's local offset.
 * A source can be rendered more than once in the same published graph. */
export const QueryLocationContext = createContext<string>();

export const queryBlockLocation = (id: string) => JSON.stringify(["block", id]);
export const queryPageLocation = (kind: string | undefined, name: string | undefined) =>
  JSON.stringify(["page", kind, name]);
