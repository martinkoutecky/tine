import { Show, createEffect, onCleanup, onMount, type JSX } from "solid-js";
import {
  activeDrawer,
  dismissMobileDrawer,
  normalizeSidebarDrawers,
  rightSidebarOpen,
  sidebarOpen,
} from "../ui";
import {
  containDrawerFocus,
  focusDrawer,
  installMobileDrawerMode,
  mobileDrawerMode,
  restoreDrawerFocus,
  trapDrawerTab,
  type DrawerDismissReason,
  type DrawerSide,
} from "../mobileDrawers";

type BackgroundSide = DrawerSide | "any";

/** A production shell region which is ordinary background for one or both
 * drawers.  Keeping this decision in a mounted component makes it difficult to
 * accidentally add aria-hidden-only isolation or leave inert behind. */
export function DrawerBackground(props: {
  blockedBy: BackgroundSide;
  class?: string;
  role?: "alert" | "status";
  ariaLive?: "off" | "polite" | "assertive";
  children: JSX.Element;
}): JSX.Element {
  const blocked = () => {
    const active = activeDrawer();
    return active != null && (props.blockedBy === "any" || props.blockedBy === active);
  };
  return (
    <div
      class={props.class}
      data-drawer-background={props.blockedBy}
      inert={blocked() ? true : undefined}
      role={props.role}
      aria-live={props.ariaLive}
    >
      {props.children}
    </div>
  );
}

/** The exact dialog/focus semantics shared by both production sidebar panels. */
export function MobileDrawerPanel(props: {
  side: DrawerSide;
  label: string;
  class: string;
  style?: JSX.CSSProperties;
  children: JSX.Element;
}): JSX.Element {
  const modal = () => mobileDrawerMode() && activeDrawer() === props.side;

  // Swipe-to-close: dragging the drawer toward its own closed edge dismisses it,
  // matching native drawers (GH: Martin). We distinguish a horizontal close-ward
  // drag from a vertical scroll on the first ~10px of movement, so scrolling
  // inside the drawer is never captured. Reuses the `back` dismiss reason (this
  // is the gesture equivalent of the OS back the drawer already honors).
  const DECIDE_PX = 10; // movement before committing to horizontal vs vertical
  const MIN_CLOSE_PX = 56; // absolute floor so a tiny panel still needs a real drag
  const CLOSE_FRACTION = 0.3; // fraction of panel width that triggers a close
  // Positive result = movement toward this side's closed edge (left closes left).
  const towardClose = (dx: number) => (props.side === "left" ? -dx : dx);
  let startX = 0;
  let startY = 0;
  let width = 1;
  let tracking = false;
  let decided = false;
  let swiping = false;
  const onTouchStart = (event: TouchEvent) => {
    if (!modal() || event.touches.length !== 1) {
      tracking = false;
      return;
    }
    const touch = event.touches[0];
    startX = touch.clientX;
    startY = touch.clientY;
    width = (event.currentTarget as HTMLElement).offsetWidth || 1;
    tracking = true;
    decided = false;
    swiping = false;
  };
  const onTouchMove = (event: TouchEvent) => {
    if (!tracking || event.touches.length !== 1) return;
    if (decided) return;
    const touch = event.touches[0];
    const dx = touch.clientX - startX;
    const dy = touch.clientY - startY;
    if (Math.abs(dx) < DECIDE_PX && Math.abs(dy) < DECIDE_PX) return;
    decided = true;
    // Only a horizontal-dominant drag toward the closed edge is a close gesture;
    // anything vertical (or opening-ward) is left to normal scrolling.
    swiping = Math.abs(dx) > Math.abs(dy) && towardClose(dx) > 0;
  };
  const onTouchEnd = (event: TouchEvent) => {
    if (!tracking) return;
    tracking = false;
    if (!swiping) return;
    swiping = false;
    const touch = event.changedTouches[0];
    if (!touch) return;
    const closeDistance = towardClose(touch.clientX - startX);
    if (closeDistance >= Math.max(MIN_CLOSE_PX, width * CLOSE_FRACTION)) {
      dismiss("back");
    }
  };
  const onTouchCancel = () => {
    tracking = false;
    swiping = false;
  };

  return (
    <div
      class={props.class}
      data-mobile-drawer-panel={props.side}
      role={modal() ? "dialog" : undefined}
      aria-modal={modal() ? "true" : undefined}
      aria-label={modal() ? props.label : undefined}
      tabindex={modal() ? -1 : undefined}
      onKeyDown={(event) => {
        if (modal() && event.key === "Tab") trapDrawerTab(event, event.currentTarget);
      }}
      onTouchStart={onTouchStart}
      onTouchMove={onTouchMove}
      onTouchEnd={onTouchEnd}
      onTouchCancel={onTouchCancel}
      style={props.style}
    >
      {props.children}
    </div>
  );
}

function dismiss(reason: DrawerDismissReason) {
  if (!dismissMobileDrawer(reason)) return false;
  restoreDrawerFocus(reason);
  return true;
}

/** Installs the single width listener, normalizes exclusivity, owns focus
 * containment, and renders the one consuming scrim.  App renders this once;
 * global transients remain siblings above it and suspend containment through
 * topTransientLayer(). */
export function MobileDrawerController(): JSX.Element {
  onMount(() => {
    const dispose = installMobileDrawerMode();
    onCleanup(dispose);
  });

  createEffect(() => {
    mobileDrawerMode();
    sidebarOpen();
    rightSidebarOpen();
    normalizeSidebarDrawers();
  });

  createEffect(() => {
    const side = activeDrawer();
    if (side) queueMicrotask(() => {
      if (activeDrawer() === side) focusDrawer(side);
    });
  });

  onMount(() => {
    const contain = () => {
      const side = activeDrawer();
      if (!side) return;
      const drawer = document.querySelector<HTMLElement>(side === "left" ? ".left-sidebar" : ".right-sidebar");
      if (drawer) containDrawerFocus(drawer);
    };
    document.addEventListener("focusin", contain, true);
    onCleanup(() => document.removeEventListener("focusin", contain, true));
  });

  return (
    <Show when={activeDrawer()}>
      <div
        class="mobile-drawer-scrim"
        data-mobile-drawer-scrim
        aria-hidden="true"
        onPointerDown={(event) => {
          event.preventDefault();
          event.stopPropagation();
        }}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          dismiss("scrim");
        }}
      />
    </Show>
  );
}

export function dismissDrawerAndRestore(reason: DrawerDismissReason) {
  return dismiss(reason);
}
