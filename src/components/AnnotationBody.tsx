import { t } from "../i18n";
import { Show, type JSX } from "solid-js";
import { InlineText } from "../render/inline";
import { formatForPage } from "../document";
import { HL_COLOR_BG, HL_COLOR_SOLID } from "../pdf";
import { openAnnotationPdf } from "../openAnnotationPdf";
import { PdfAreaImage } from "./PdfAreaImage";

// The colored, clickable swatch rendered for a PDF highlight (annotation) block:
// a dot + page badge (click → open the PDF at that page) and the highlight text.
export function AnnotationBody(props: {
  highlightId: string;
  color: string;
  hlPage: number;
  line: string;
  page: string;
  areaStamp?: string;
}): JSX.Element {
  const openHighlightPdf = (e: MouseEvent) => {
    e.stopPropagation();
    openAnnotationPdf(props.page, props.hlPage, props.highlightId);
  };
  return (
    <span class="pdf-annotation-line">
      <span
        class="hl-prefix"
        data-highlight-id={props.highlightId}
        onClick={openHighlightPdf}
        title={t("annotation-body-open-in-pdf-p", { pageNumber: String(props.hlPage) })}
      >
        <span class="hl-dot" style={{ background: HL_COLOR_SOLID[props.color] ?? HL_COLOR_SOLID.yellow }} />
        <strong class="hl-page-badge">P{props.hlPage}</strong>
      </span>{" "}
      <span class="hl-text" style={{ background: HL_COLOR_BG[props.color] ?? HL_COLOR_BG.yellow }}>
        <InlineText text={props.line} format={formatForPage(props.page)} />
      </span>
      <Show when={props.areaStamp}>
        {stamp => <PdfAreaImage page={props.page} highlightId={props.highlightId} hlPage={props.hlPage} stamp={stamp()} onClick={openHighlightPdf} />}
      </Show>
    </span>
  );
}
