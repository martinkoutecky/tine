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
