import type { JSX } from "solid-js";
import { PageRef } from "../render/inline";

/** The header line of a whole-page `{{embed [[Page]]}}`: a page glyph and the
 *  page's link, the indicator that this is a page and not a block (GH #439; OG
 *  `block.cljs` `page-embed`: `.embed-header` = svg/page + `page-cp`). The link is
 *  the ordinary page reference, so its click gestures, hover peek and context
 *  menu are the app's one set. */
export function PageEmbedHeader(props: { name: string; blockId?: string }): JSX.Element {
  return (
    <div class="embed-page-header">
      <svg class="embed-page-glyph" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zm-1 7V3.5L18.5 9H13z" />
      </svg>
      <PageRef name={props.name} blockId={props.blockId} />
    </div>
  );
}
