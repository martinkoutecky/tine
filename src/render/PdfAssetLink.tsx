import { openPdf } from "../panes";
import { pdfAssetFile } from "../editor/annotation";
import { isQuickShareViewer } from "../publishedBackend";
import { ShareMedia } from "../quickShare/ShareMedia";
import type { SpanDomAttrs } from "./spans";

/** The label is the basename; the route retains the whole path under assets/. */
export function pdfFilenameFromDest(dest: string): string {
  const path = dest.replace(/\\/g, "/");
  return path.split("/").pop() || path;
}

export function PdfAssetLink(props: { dest: string; label?: string; spanAttrs?: SpanDomAttrs }) {
  if (isQuickShareViewer()) return <ShareMedia url={props.dest} label={props.label} />;
  const filename = pdfAssetFile(props.dest);
  const label = props.label || pdfFilenameFromDest(props.dest);
  return <a class="external-link pdf-link" {...(props.spanAttrs ?? {})}
    onClick={(e) => { e.stopPropagation(); openPdf(filename, label); }}>📄 {label}</a>;
}
