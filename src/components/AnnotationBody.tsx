import { type JSX } from "solid-js";
import { openPdf } from "../ui";
import { InlineText } from "../render/inline";
import { formatForPage } from "../store";
import { HL_COLOR_BG, HL_COLOR_SOLID } from "../pdf";
import { pdfFileForPage } from "../editor/annotation";

// The colored, clickable swatch rendered for a PDF highlight (annotation) block:
// a dot + page badge (click → open the PDF at that page) and the highlight text.
export function AnnotationBody(props: {
  highlightId: string;
  color: string;
  hlPage: number;
  line: string;
  page: string;
}): JSX.Element {
  const openHighlightPdf = (e: MouseEvent) => {
    e.stopPropagation();
    const file = pdfFileForPage(props.page);
    if (file) openPdf(file, file, props.hlPage, props.highlightId);
  };
  return (
    <span class="pdf-annotation-line">
      <span
        class="hl-prefix"
        data-highlight-id={props.highlightId}
        onClick={openHighlightPdf}
        title={`Open in PDF (P${props.hlPage})`}
      >
        <span class="hl-dot" style={{ background: HL_COLOR_SOLID[props.color] ?? HL_COLOR_SOLID.yellow }} />
        <strong class="hl-page-badge">P{props.hlPage}</strong>
      </span>{" "}
      <span class="hl-text" style={{ background: HL_COLOR_BG[props.color] ?? HL_COLOR_BG.yellow }}>
        <InlineText text={props.line} format={formatForPage(props.page)} />
      </span>
    </span>
  );
}
