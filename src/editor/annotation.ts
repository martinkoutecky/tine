// PDF highlight (annotation) blocks — `ls-type:: annotation` blocks Logseq writes
// for a PDF highlight. Their raw is generated metadata, so the editor shows only
// the highlight text and clicking the swatch jumps to the PDF. Detection + the
// bits the renderer needs, kept out of Block.tsx.

import { doc } from "../store";

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
  const hlPage = Number(properties.find(([k]) => k === "hl-page")?.[1] ?? "1");
  return { color, hlPage };
}

/** Resolve the PDF filename for an annotation block from its owning hls__ page's
 *  `file-path::` property. */
export function pdfFileForPage(pageName: string): string | null {
  const p = doc.pages.find((x) => x.name === pageName);
  const m = p?.preBlock ? /file-path::\s*(\S+)/.exec(p.preBlock) : null;
  return m ? m[1].split("/").pop() ?? null : null;
}
