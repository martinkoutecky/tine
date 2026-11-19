import { Show, type JSX } from "solid-js";
import { pageOpening, type FeedPage } from "../document";
import { t } from "../i18n";
import { OpeningText } from "../render/openingText";
import { BlockRenderBoundary } from "./BlockRenderBoundary";

/** A page's opening text, the prose before its first block (GH #85), as the
 *  row a click turns into an editable block (`onEdit`). Its text is
 *  {@link pageOpening}'s: where the header ends is the parser's answer, kept
 *  while the header is edited. */
export function PageOpeningRow(props: { page: FeedPage; onEdit: () => void }): JSX.Element {
  return (
    <Show when={pageOpening(props.page).content}>
      {(content) => (
        <div class="ls-block preamble-block" data-page-preamble={props.page.name}>
          <div class="block-main">
            <div class="block-controls">
              <span class="collapse-toggle" />
              <span class="bullet-container" title={t("page-ui-click-the-text-to-turn-it-into-an-editable")}>
                <span class="bullet" />
              </span>
            </div>
            <div class="block-content-wrapper" onClick={() => props.onEdit()}>
              <BlockRenderBoundary raw={content()}><div class="block-content"><OpeningText text={content()} format={props.page.format} /></div></BlockRenderBoundary>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}
