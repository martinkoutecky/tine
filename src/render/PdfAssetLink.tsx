import { openPdf } from "../panes";
import { isQuickShareViewer } from "../publishedBackend";
import { ShareLocalMedia } from "../quickShare/ShareMedia";
import type { SpanDomAttrs } from "./spans";

export function PdfAssetLink(props: { filename: string; label?: string; spanAttrs?: SpanDomAttrs }) {
  if (isQuickShareViewer()) return <ShareLocalMedia path={props.filename} label={props.label} />;
  const label = props.label || props.filename.split("/").pop() || props.filename;
  return <a class="external-link pdf-link" {...(props.spanAttrs ?? {})}
    onClick={(e) => { e.stopPropagation(); openPdf(props.filename, label); }}>📄 {label}</a>;
}
