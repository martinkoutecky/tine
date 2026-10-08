// The image viewer's gallery (GH #501). OG's open-lightbox
// (components/block.cljs L244-262) gathers every `.asset-container img` in the
// document, sorts them reading-order by (y, x), and starts the gallery AT the
// clicked image followed by the rest, then the ones that came before it
// (split-with / reverse / concat). Tine mirrors that over the images that are
// currently rendered: `img.inline-image`. Virtualised blocks that are not in
// the DOM are not in the gallery - the viewer swipes between what is on the page.

export interface GalleryItem { src: string; x: number; y: number; key: unknown }

/** Reading order, rotated so `clicked` is first. Pure. */
export function orderGallery(items: GalleryItem[], clicked: unknown): GalleryItem[] {
  if (items.length <= 1) return items.slice();
  const sorted = items.slice().sort((a, b) => a.y - b.y || a.x - b.x);
  const at = sorted.findIndex((i) => i.key === clicked);
  if (at < 0) return sorted;
  return [...sorted.slice(at), ...sorted.slice(0, at)];
}

/** The srcs of the rendered inline images, starting at `clicked`. */
export function galleryFor(clicked: HTMLElement, root: ParentNode = document): string[] {
  const items: GalleryItem[] = [];
  root.querySelectorAll<HTMLImageElement>("img.inline-image").forEach((img) => {
    const src = img.currentSrc || img.getAttribute("src") || "";
    if (!src || !img.isConnected) return;
    const r = img.getBoundingClientRect();
    items.push({ src, x: r.x, y: r.y, key: img });
  });
  return orderGallery(items, clicked).map((i) => i.src);
}

/** Next index when turning the page by `delta`: wraps with 3+ images (PhotoSwipe
 *  `loop`), stops at the ends with 2, nothing with 1. null = no such image. */
export function stepIndex(index: number, count: number, delta: 1 | -1): number | null {
  if (count <= 1) return null;
  const next = index + delta;
  if (count >= 3) return (next + count) % count;
  return next >= 0 && next < count ? next : null;
}
