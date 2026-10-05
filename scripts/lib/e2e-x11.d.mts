export interface WindowGeometry { WINDOW: number; X: number; Y: number; WIDTH: number; HEIGHT: number }
export interface FrameExtents { left: number; right: number; top: number; bottom: number }
/** Client origin and size from `xwininfo -id` output. */
export function parseXwininfo(raw: string, id: string | number): WindowGeometry;
/** Frame extents from `xprop` output; an absent property is zero only when asked. */
export function parseFrameExtents(raw: string, options?: { missingAsZero?: boolean }): FrameExtents;
/** Window ids in descending area (unreadable geometry keeps order). */
export function largestFirst(ids: string[], geometryOf: (id: string) => { WIDTH: number; HEIGHT: number }): string[];
/** X11 helpers bound to a journey's environment. */
export function x11Tools(env: NodeJS.ProcessEnv, options?: { xdotool?: string }): {
  xdo: (...args: string[]) => string;
  geometry: (id: string) => WindowGeometry;
  windowIds: () => string[];
  frameExtents: (id: string, options?: { missingAsZero?: boolean }) => FrameExtents;
};
