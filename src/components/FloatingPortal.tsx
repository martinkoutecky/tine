import { Portal } from "solid-js/web";
import type { JSX } from "solid-js";

/**
 * **The ONE way a component floats a surface out of its own DOM subtree.**
 *
 * Solid's `<Portal>` mounts under `document.body`, but it also sets
 * `container._$host = marker.parentNode`, so Solid's delegated events (`click`,
 * `mousedown`, `pointerdown`, `keydown`, `contextmenu`, …) keep bubbling through
 * the LOGICAL parent chain after leaving the real one. A press on a portalled
 * surface is then delivered to every handler on the block that rendered it —
 * `.block-content-wrapper`'s `onMouseDown` arms an edit gesture and the block's
 * own `onKeyDown`/`onContextMenu` see the key and the right-click — which swaps
 * the surface out from under the user (GH #619: pressing anything in the open
 * query sheet dropped the block into raw `{{query }}` text).
 *
 * This wrapper severs that chain: after the portal mounts, `_$host` reads
 * `undefined`, so delegation continues through the container's real parent
 * (`body`), exactly as it would for a node appended by hand. Nothing in the app
 * relies on a portal's events reaching its logical parent (dismissal, focus and
 * layer ownership are DOM-based), and a surface that WANTS its owner to hear an
 * event must call that owner explicitly — it may not inherit the whole bubble.
 *
 * Exemplar of the rule; `portalBoundary.guard.test.ts` fails on a direct
 * `Portal` import from `solid-js/web` anywhere else.
 */
export function FloatingPortal(props: { mount?: Node; children: JSX.Element; ref?: (container: HTMLDivElement) => void }): JSX.Element {
  return (
    <Portal
      mount={props.mount}
      ref={(container) => {
        // Own property on the container; Solid's `get()` accessor is configurable.
        Object.defineProperty(container, "_$host", { value: undefined, configurable: true });
        props.ref?.(container);
      }}
    >
      {props.children}
    </Portal>
  );
}
