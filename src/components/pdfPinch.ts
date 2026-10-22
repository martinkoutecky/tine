export type PinchPoint = { identifier: number; clientX: number; clientY: number };
export type PinchPreview = { scale: number; factor: number; origin: { x: number; y: number }; midpoint: { x: number; y: number } };

/** Touch events retain native single-finger scrolling and selection. Only a
 * two-finger sequence is claimed; after either finger lifts it cannot restart
 * until every finger is up. No scale state or raster changes during preview. */
export function createPdfPinch(options: {
  scale: () => number;
  start: (origin: { x: number; y: number }) => void;
  preview: (value: PinchPreview) => void;
  finish: (value: PinchPreview | null) => void;
}) {
  let gesture: { ids: number[]; distance: number; scale: number; origin: { x: number; y: number } } | null = null;
  let value: PinchPreview | null = null;
  let claimed = false;
  let suppressUntil = 0;
  const geometry = (a: PinchPoint, b: PinchPoint) => ({
    distance: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY),
    midpoint: { x: (a.clientX + b.clientX) / 2, y: (a.clientY + b.clientY) / 2 },
  });
  function finish(commit: boolean) {
    if (!gesture) return;
    gesture = null;
    options.finish(commit ? value : null);
    value = null;
    suppressUntil = Date.now() + 400; // selectionchange / compatibility clicks after release
  }
  return {
    suppressed: () => claimed || Date.now() < suppressUntil,
    update(points: readonly PinchPoint[], phase: "start" | "move" | "end" | "cancel") {
      if (phase === "cancel") {
        const consumed = claimed;
        finish(false);
        claimed = consumed && points.length > 0;
        return consumed;
      }
      if (!claimed && phase === "start" && points.length === 1) suppressUntil = 0;
      if (!claimed && phase === "start" && points.length === 2) {
        const { distance, midpoint } = geometry(points[0], points[1]);
        if (distance <= 0) return false;
        claimed = true;
        gesture = { ids: points.map((p) => p.identifier), distance, scale: options.scale(), origin: midpoint };
        value = { scale: gesture.scale, factor: 1, origin: midpoint, midpoint };
        options.start(midpoint);
      }
      const consumed = claimed;
      if (gesture) {
        const a = points.find((p) => p.identifier === gesture!.ids[0]);
        const b = points.find((p) => p.identifier === gesture!.ids[1]);
        if (points.length > 2) finish(false);
        else if (!a || !b) finish(true);
        else if (phase === "move") {
          const { distance, midpoint } = geometry(a, b);
          const scale = Math.min(4, Math.max(0.4, gesture.scale * distance / gesture.distance));
          value = { scale, factor: scale / gesture.scale, origin: gesture.origin, midpoint };
          options.preview(value);
        }
      }
      if (points.length === 0) claimed = false;
      return consumed;
    },
    cancel() { finish(false); claimed = false; },
  };
}
