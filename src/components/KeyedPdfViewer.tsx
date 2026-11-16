import { Show, type JSX } from "solid-js";
import type { PdfRoute } from "../router";
import { pdfNavigationIntent } from "../pdfNavigation";
import { pdfOwnershipKey, type PdfOwnership } from "../pdfOwnership";
import type { PdfTarget } from "./pdfViewerPrimitives";
import { PdfViewer } from "./PdfViewer";

/**
 * A view (viewId) is the mount identity, not a navigation request: page and
 * highlight changes within one view stay reactive, while switching views or
 * assets tears down every view-local cache and pdf.js task. Views of the same
 * file share only their highlights (acquirePdfHighlightState).
 */
export function KeyedPdfViewer(props: {
  route: () => PdfRoute | null;
  owner: () => PdfOwnership | null;
  focused?: () => boolean;
  onClose?: () => void;
  onOpenNotes?: (block?: string) => void;
  onViewState?: (state: { page: number; scale: number }) => void;
  onAnotherView?: (state: { page: number; scale: number }) => void;
}): JSX.Element {
  const resolvedTarget = (): PdfTarget | null => {
    const route = props.route();
    const owner = props.owner();
    if (!route || !owner) return null;
    const intent = pdfNavigationIntent(route.viewId)();
    return { filename: route.filename, label: route.label, owner,
      page: intent?.page ?? route.page,
      ...(route.scale !== undefined ? { scale: route.scale } : {}),
      ...(intent?.highlightId ? { highlightId: intent.highlightId } : {}) };
  };
  const resourceKey = () => {
    const target = resolvedTarget();
    return target ? `${pdfOwnershipKey(target.owner)}:${props.route()!.viewId}` : null;
  };
  return (
    <Show when={resourceKey()} keyed>
      {(_key) => {
        const target = resolvedTarget()!;
        return (
          <PdfViewer
            filename={target.filename}
            label={resolvedTarget()?.label ?? target.filename}
            owner={target.owner}
            page={resolvedTarget()?.page}
            scale={resolvedTarget()?.scale}
            navigation={resolvedTarget}
            navigationKey={() => {
              const route = props.route();
              return route ? `intent:${pdfNavigationIntent(route.viewId)()?.serial ?? 0}` : "none";
            }}
            focused={props.focused}
            onClose={props.onClose}
            onOpenNotes={props.onOpenNotes}
            onViewState={(state) => { if (resourceKey() === _key) props.onViewState?.(state); }}
            onAnotherView={props.onAnotherView}
          />
        );
      }}
    </Show>
  );
}
