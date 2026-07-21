import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { registerTransientLayer } from "../transientLayers";

export interface TopbarOverflowMenuProps {
  onCalendar: () => void;
  onJournals: () => void;
  onToggleTheme: () => void;
  onToggleRightSidebar: (trigger?: HTMLElement | null) => void;
  onBack: () => void;
  onForward: () => void;
  canGoBack: () => boolean;
  canGoForward: () => boolean;
}

/**
 * Low-priority topbar actions at compact widths. The toolbar's container query
 * only changes which controls are visible; this component owns the same actions
 * and lets the shared transient registry dismiss its popover.
 */
export function TopbarOverflowMenu(props: TopbarOverflowMenuProps): JSX.Element {
  let root: HTMLDivElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  let menu: HTMLDivElement | undefined;
  const [open, setOpen] = createSignal(false);

  const close = (restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) trigger?.focus();
  };

  createEffect(() => {
    if (!open()) return;
    queueMicrotask(() => menu?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus());
    const unregister = registerTransientLayer({
      id: "topbar-overflow",
      root: () => root ?? null,
      trigger: () => trigger ?? null,
      dismiss: () => { close(true); return true; },
    });
    onCleanup(unregister);
    // Dismiss when the user clicks anywhere outside the menu (the toolbar, the
    // page, another control). The transient registry only handles Escape/Back;
    // without this the popover stayed open on an outside click. Capture phase so
    // it fires regardless of downstream stopPropagation; focus is NOT restored to
    // the trigger here because the user's intent was to click elsewhere.
    const onDown = (e: MouseEvent) => {
      if (root && !root.contains(e.target as Node)) close(false);
    };
    document.addEventListener("mousedown", onDown, true);
    onCleanup(() => document.removeEventListener("mousedown", onDown, true));
  });

  const run = (action: () => void) => {
    // Restore focus to the "···" trigger before invoking the action. This is
    // correct menu a11y on its own, and it also makes the trigger the opener an
    // action-opened overlay (e.g. the right sidebar drawer) restores focus to on
    // close — instead of focus falling back to the document body (GH #205).
    close(true);
    action();
  };

  return (
    <div class="topbar-overflow" ref={root}>
      <button
        ref={trigger}
        type="button"
        class="icon-btn topbar-overflow-trigger"
        title="More toolbar actions"
        aria-label="More toolbar actions"
        aria-haspopup="menu"
        aria-expanded={open()}
        data-topbar-overflow-trigger
        onClick={() => setOpen((value) => !value)}
      >···</button>
      <Show when={open()}>
        <div ref={menu} class="topbar-overflow-menu" role="menu" aria-label="More toolbar actions">
          <button type="button" role="menuitem" data-topbar-overflow-action="calendar" onClick={() => run(props.onCalendar)}>Calendar</button>
          <button type="button" role="menuitem" data-topbar-overflow-action="journals" onClick={() => run(props.onJournals)}>Journals</button>
          <button type="button" role="menuitem" data-topbar-overflow-action="theme" onClick={() => run(props.onToggleTheme)}>Toggle theme</button>
          <button type="button" role="menuitem" data-topbar-overflow-action="right-sidebar" onClick={() => run(() => props.onToggleRightSidebar(trigger))}>Toggle right sidebar</button>
          <div class="topbar-overflow-sep" role="separator" />
          <button type="button" role="menuitem" data-topbar-overflow-action="back" disabled={!props.canGoBack()} onClick={() => run(props.onBack)}>Back</button>
          <button type="button" role="menuitem" data-topbar-overflow-action="forward" disabled={!props.canGoForward()} onClick={() => run(props.onForward)}>Forward</button>
        </div>
      </Show>
    </div>
  );
}
