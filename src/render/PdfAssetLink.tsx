import { openPdf } from "../panes";
import { isQuickShareViewer } from "../publishedBackend";
import { ShareLocalMedia } from "../quickShare/ShareMedia";
import type { SpanDomAttrs } from "./spans";

/** The label is the basename; the route retains the whole path under assets/. */
export function pdfFilenameFromDest(dest: string): string {
  const path = dest.replace(/\\/g, "/");
  return path.split("/").pop() || path;
}

export function PdfAssetLink(props: { filename: string; label?: string; spanAttrs?: SpanDomAttrs }) {
  if (isQuickShareViewer()) return <ShareLocalMedia path={props.filename} label={props.label} />;
  const label = props.label || pdfFilenameFromDest(props.filename);
  return <a class="external-link pdf-link" {...(props.spanAttrs ?? {})}
    onClick={(e) => { e.stopPropagation(); openPdf(props.filename, label); }}>📄 {label}</a>;
}
