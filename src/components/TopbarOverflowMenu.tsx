import { render, t } from "../i18n";
import { For, Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { toolbarActionLabel, type ToolbarActionId } from "../toolbarChoice";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";

export interface TopbarOverflowMenuProps {
  actions: ToolbarActionId[];
  visible: boolean;
  onWorkspaces?: () => void;
  onCustomize?: () => void;
  onCalendar: () => void;
  onJournals: (event: MouseEvent) => void;
  /** Share the current page; absent where sharing does not apply (a published export). */
  onShare?: () => void;
  canShare?: () => boolean;
  onToggleTheme: () => void;
  onToggleRightSidebar: (trigger?: HTMLElement | null) => void;
  onBack: () => void;
  onForward: () => void;
  canGoBack: () => boolean;
  canGoForward: () => boolean;
  /** Absent where Settings does not apply (a published export). */
  onSettings?: () => void;
}

/**
 * Optional rows come from the toolbar placement owner, in its menu order.
 * Fixed navigation retains its container tiers. Shared transient dismissal.
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
  });
  // `root` already contains the trigger. Focus is NOT restored here, unlike the
  // Escape/Back dismissal above: the user's intent was to click elsewhere.
  dismissOnOutsidePointer({ open, inside: () => [root], dismiss: () => close(false) });

  const run = (action: () => void) => {
    // Restore focus to the "···" trigger before invoking the action. This is
    // correct menu a11y on its own, and it also makes the trigger the opener an
    // action-opened overlay (e.g. the right sidebar drawer) restores focus to on
    // close — instead of focus falling back to the document body (GH #205).
    close(true);
    action();
  };
  const handlers = () => ({ calendar: props.onCalendar, journals: props.onJournals,
    share: props.onShare, theme: props.onToggleTheme,
    "right-sidebar": () => props.onToggleRightSidebar(trigger),
    workspaces: props.onWorkspaces, settings: props.onSettings });
  const rows = () => props.actions.filter(id => handlers()[id]);

  return (
    <div class="topbar-overflow" ref={root} style={{ display: props.visible ? "block" : "none" }}>
      <button
        ref={trigger}
        type="button"
        class="icon-btn topbar-overflow-trigger"
        title={t("topbar-overflow-menu-more-toolbar-actions")}
        aria-label={t("topbar-overflow-menu-more-toolbar-actions")}
        aria-haspopup="menu"
        aria-expanded={open()}
        data-topbar-overflow-trigger
        onClick={() => setOpen((value) => !value)}
      >···</button>
      <Show when={open()}>
        <div ref={menu} class="topbar-overflow-menu" role="menu" aria-label={t("topbar-overflow-menu-more-toolbar-actions")}>
          <For each={rows()}>{id => <button type="button" role="menuitem" data-topbar-overflow-action={id}
            disabled={id === "share" && props.canShare ? !props.canShare() : false}
            onClick={event => run(() => handlers()[id]!(event))}>{render(toolbarActionLabel(id))}</button>}</For>
          <div class="topbar-overflow-sep" role="separator" />
          <button type="button" role="menuitem" data-topbar-overflow-action="back" disabled={!props.canGoBack()} onClick={() => run(props.onBack)}>{t("topbar-overflow-menu-back")}</button>
          <button type="button" role="menuitem" data-topbar-overflow-action="forward" disabled={!props.canGoForward()} onClick={() => run(props.onForward)}>{t("topbar-overflow-menu-forward")}</button>
          <Show when={props.onCustomize}>
            <div class="topbar-overflow-settings-sep" role="separator" />
            <button type="button" role="menuitem" onClick={() => run(props.onCustomize!)}>{t("toolbar-customize")}</button>
          </Show>
        </div>
      </Show>
    </div>
  );
}
