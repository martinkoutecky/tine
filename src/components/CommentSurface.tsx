import { useContext, type JSX } from "solid-js";
import type { FeedPage } from "../document";
import { SurfaceContext } from "./Block";
import { createMarginSurface } from "./MarginColumn";
import { MarginContext } from "./marginContext";
import { CommentResolutionScope } from "./CommentResolutionScope";

/** Secondary outlines measure their own width, using exactly the page tier
 * policy. Nested embeds own their margin instead of borrowing their host's. */
export function CommentSurface(props: { page: FeedPage | undefined; roots?: readonly string[]; main?: boolean; children: JSX.Element }): JSX.Element {
  const surface = useContext(SurfaceContext);
  let pane: HTMLDivElement | undefined;
  let outline: HTMLDivElement | undefined;
  const margin = createMarginSurface({
    page: () => props.page && (props.roots ? { ...props.page, roots: [...props.roots] } : props.page),
    eligible: () => true, pane: props.main ? undefined : () => pane, outline: () => outline,
  });
  return <div class={props.main ? "main-comment-surface" : "comment-surface"} ref={pane}>
    <div class="page-section">
      <CommentResolutionScope roots={props.roots ?? props.page?.roots ?? []}>
      <MarginContext.Provider value={{ ...margin.placement, surface }}>
        <div class="page-blocks" ref={outline} classList={{ "margin-active": !!props.main && margin.active(), "comment-margin-active": !props.main && margin.active() }}>{props.children}</div>
      </MarginContext.Provider>
      <margin.Column />
      </CommentResolutionScope>
    </div>
  </div>;
}
