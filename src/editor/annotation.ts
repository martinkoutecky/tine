// PDF highlight (annotation) blocks — `ls-type:: annotation` blocks Logseq writes
// for a PDF highlight. Their raw is generated metadata, so the editor shows only
// the highlight text and clicking the swatch jumps to the PDF. Detection + the
// bits the renderer needs, kept out of Block.tsx.

import { readPageProperty, pageHeaderProperties } from "../document";
import type { BlockDto, Format } from "../types";
import { facetsOf } from "../render/facets";

/** True for a PDF highlight (annotation) block. */
export function isAnnotationBlock(raw: string, format: Format = "md"): boolean {
  return annotationProperty(facetsOf(raw, format).properties) !== undefined;
}

function annotationProperty(properties: [string, string][]): [string, string] | undefined {
  return properties.find(([key, value]) => key === "ls-type" && value === "annotation");
}

/** Highlight colour + 1-based PDF page from a block's parsed properties; null if
 *  the block isn't an annotation. */
export function annotationInfo(
  properties: [string, string][]
): { color: string; hlPage: number } | null {
  if (!annotationProperty(properties)) return null;
  const color = properties.find(([k]) => k === "hl-color")?.[1] ?? "yellow";
  const parsedPage = Number(properties.find(([k]) => k === "hl-page")?.[1] ?? "1");
  const hlPage = Number.isFinite(parsedPage) && parsedPage >= 1 ? Math.floor(parsedPage) : 1;
  return { color, hlPage };
}

/** Parsed DTO properties, including an empty result, are authoritative. Missing
 * facets use the same cached parser as live blocks (O(properties) warm). */
export function annotationInfoForBlock(block: Pick<BlockDto, "raw" | "properties">, format: Format = "md"): {
  color: string; hlPage: number;
} | null {
  return annotationInfo(block.properties ?? facetsOf(block.raw, format).properties);
}

/** Resolve a page pre-block's PDF path to the asset basename. Logseq writes
 * `file-path::` in Markdown and `#+FILE-PATH:` in Org; both values extend to
 * end-of-line so filenames containing spaces remain usable. */
export function pdfFileFromPreBlock(preBlock: string | null | undefined, format: Format = "md"): string | null {
  const properties = pageHeaderProperties({ name: "", kind: "page", title: "",
    preBlock: preBlock ?? null, roots: [], format, readOnly: true, guide: false });
  return pdfBasename(properties.find(([key]) => key === "file-path")?.[1]);
}

function pdfBasename(path: string | null | undefined): string | null {
  if (!path) return null;
  return path.split(/[\\/]/).pop() || null;
}

/** Resolve the PDF filename for an annotation block from its owning hls__ page's
 *  `file-path::` property. */
export function pdfFileForPage(pageName: string): string | null {
  return pdfBasename(readPageProperty(pageName, "file-path"));
}
