import { Annotation } from "@codemirror/state";

/** Optional platform input data when a transaction represents a browser draft
 * replacement. Without it, the inserted transaction text is the input. */
export const cm6InputData = Annotation.define<string | null>();
