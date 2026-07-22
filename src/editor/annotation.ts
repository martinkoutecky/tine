// PDF highlight (annotation) blocks — `ls-type:: annotation` blocks Logseq writes
// for a PDF highlight. Their raw is generated metadata, so the editor shows only
// the highlight text and clicking the swatch jumps to the PDF. Detection + the
// bits the renderer needs, kept out of Block.tsx.

import { doc } from "../store";
import type { BlockDto } from "../types";

/** True for a PDF highlight (annotation) block. */
export function isAnnotationBlock(raw: string): boolean {
  return /^\s*ls-type::\s*annotation\s*$/m.test(raw);
}

/** Highlight colour + 1-based PDF page from a block's parsed properties; null if
 *  the block isn't an annotation. */
export function annotationInfo(
  properties: [string, string][]
): { color: string; hlPage: number } | null {
  if (!properties.some(([k, v]) => k === "ls-type" && v === "annotation")) return null;
  const color = properties.find(([k]) => k === "hl-color")?.[1] ?? "yellow";
  const parsedPage = Number(properties.find(([k]) => k === "hl-page")?.[1] ?? "1");
  const hlPage = Number.isFinite(parsedPage) && parsedPage >= 1 ? Math.floor(parsedPage) : 1;
  return { color, hlPage };
}

/** Annotation metadata from a backend block DTO. Real backends provide parsed
 * properties; the raw fallback also covers old/mock DTOs without those facets. */
export function annotationInfoForBlock(block: Pick<BlockDto, "raw" | "properties">): {
  color: string;
  hlPage: number;
} | null {
  const parsed = annotationInfo(block.properties ?? []);
  if (parsed) return parsed;
  if (!isAnnotationBlock(block.raw)) return null;
  const color = /^\s*hl-color::\s*(.*?)\s*$/m.exec(block.raw)?.[1] || "yellow";
  const parsedPage = Number(/^\s*hl-page::\s*(.*?)\s*$/m.exec(block.raw)?.[1] ?? "1");
  const hlPage = Number.isFinite(parsedPage) && parsedPage >= 1 ? Math.floor(parsedPage) : 1;
  return { color, hlPage };
}

/** Resolve a page pre-block's PDF path to the asset basename. Logseq writes
 * `file-path::` in Markdown and `#+FILE-PATH:` in Org; both values extend to
 * end-of-line so filenames containing spaces remain usable. */
export function pdfFileFromPreBlock(preBlock: string | null | undefined): string | null {
  const source = preBlock ?? "";
  const path = /^\s*file-path::\s*(.*?)\s*$/mi.exec(source)?.[1]
    ?? /^\s*#\+file-path:\s*(.*?)\s*$/mi.exec(source)?.[1];
  if (!path) return null;
  return path.split(/[\\/]/).pop() || null;
}

/** Resolve the PDF filename for an annotation block from its owning hls__ page's
 *  `file-path::` property. */
export function pdfFileForPage(pageName: string): string | null {
  const p = doc.pages.find((x) => x.name === pageName);
  return pdfFileFromPreBlock(p?.preBlock);
}
