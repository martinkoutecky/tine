import { createContext, type Accessor } from "solid-js";
import type { Block, Span } from "./ast";

export type CodeEditAction = Accessor<((sourceByte: number) => void) | undefined>;

/** While a page's opening text renders (D-20261119-PG): what of it shows as
 *  its exact source rather than as rendered nodes. */
export interface OpeningSource {
  /** The exact source of a property node or a top-level separator, from the
   *  end of `prev` (or its line start); undefined for a node rendered as usual. */
  literal(node: Block, prev: Block | undefined): string | undefined;
  /** The exact source of an inline node, or undefined when the span is not
   *  this text's (a reference or macro expansion shown inside it). */
  slice(span: Span, starts: string): string | undefined;
}

/** The text a body renders. Every `AstBody` body provides its own; only an
 *  opening body sets `opening`. */
export const BodySourceContext = createContext<{ raw?: string; codeEditAction?: CodeEditAction; opening?: OpeningSource }>();
