import { type JSX } from "solid-js";
import { AssetImage } from "../render/inline";
import "./PdfAreaImage.css";

/** OG: pdf/assets.cljs area-display + publishing/db.cljs get-area-block-asset-url.
 * Pass the raw asset name to the ordinary image resolver (not an encoded URL).
 * Its bounded cache, backend path checks and sync invalidation apply unchanged.
 * Tine retains its annotation chrome without OG's copy/maximize toolbar;
 * image click follows the annotation, as OG's prefix-link does. */
export function PdfAreaImage(props: {
  page: string;
  highlightId: string;
  hlPage: number;
  stamp: string;
  onClick?: (event: MouseEvent) => void;
}): JSX.Element {
  const path = () => `../assets/${props.page.replace(/^hls__/, "")}/${props.hlPage}_${props.highlightId}_${props.stamp}.png`;
  return <span class="pdf-area-image" onMouseDown={e => e.stopPropagation()}>
    <AssetImage url={path()} alt="" onClick={e => props.onClick?.(e)} />
  </span>;
}
