import { For, createMemo, createSignal, onCleanup } from "solid-js";
import { render, t } from "../i18n";
import { saveToolbarChoice, toolbarActions, toolbarActionLabel, toolbarChoice, type ToolbarActionId, type ToolbarChoice } from "../toolbarChoice";
import { ToolbarActionIcon } from "./ToolbarActionIcon";
import { settings } from "./settingDefinitions";
import { settingAttributes } from "./settingsField";
import { beginRowReorderDrag, rowReorderClickSuppressed, type RowDropTarget } from "./rowReorder";

/** Existing Appearance pane, with pointer dragging and equivalent keyboard /
 * touch buttons. Each move persists one ordered two-list device record. */
export function ToolbarSettings() {
  const lists = createMemo<ToolbarChoice>(() => toolbarChoice() ?? { bar: toolbarActions.map(action => action.id), menu: [] });
  const [target, setTarget] = createSignal<RowDropTarget | null>(null);
  let cancelDrag: (() => void) | undefined;
  onCleanup(() => cancelDrag?.());
  const move = (id: ToolbarActionId, section: keyof ToolbarChoice, index: number) => {
    if (lists()[section][index] === id) return;
    const next = { bar: lists().bar.filter(value => value !== id), menu: lists().menu.filter(value => value !== id) };
    next[section].splice(index, 0, id);
    saveToolbarChoice(next);
  };
  const drag = (event: PointerEvent, id: ToolbarActionId, section: keyof ToolbarChoice) => {
    cancelDrag?.();
    const snapshot = lists();
    cancelDrag = beginRowReorderDrag(event, ".toolbar-settings [data-toolbar-drop-section]", setTarget, target => {
      if (lists() !== snapshot) return;
      const destination = target.row.dataset.toolbarDropSection as keyof ToolbarChoice;
      let slot = target.index + (target.row.tagName === "LI" && !target.before ? 1 : 0);
      if (destination === section && snapshot[section].indexOf(id) < slot) slot--;
      move(id, destination, slot);
    });
  };
  const clickMove = (id: ToolbarActionId, section: keyof ToolbarChoice, index: number) => {
    if (!rowReorderClickSuppressed()) move(id, section, index);
  };
  return <section class="toolbar-settings" {...settingAttributes(settings["toolbar-title"])}>
    <h3>{t("toolbar-title")}</h3>
    <p>{t("toolbar-description")}</p>
    <For each={["bar", "menu"] as const}>{section => <div class="toolbar-settings-section"
      data-toolbar-section={section} data-toolbar-drop-section={section} data-row-index={lists()[section].length}>
      <h4>{t(section === "bar" ? "toolbar-on-bar" : "toolbar-in-menu")}</h4>
      <ul>
        <For each={lists()[section]}>{(id, index) => {
          const label = () => render(toolbarActionLabel(id));
          return <li data-toolbar-choice={id} data-toolbar-drop-section={section} data-row-index={index()}
            classList={{ "toolbar-drop-target": target()?.row.dataset.toolbarChoice === id }}>
            <span class="toolbar-drag-handle" onPointerDown={event => drag(event, id, section)} aria-hidden="true">⠿</span>
            <ToolbarActionIcon id={id} /><span class="toolbar-choice-name">{label()}</span>
            <button type="button" class="settings-btn" disabled={index() === 0} aria-label={t("toolbar-move-up", { action: label() })} onClick={() => clickMove(id, section, index() - 1)}>↑</button>
            <button type="button" class="settings-btn" disabled={index() === lists()[section].length - 1} aria-label={t("toolbar-move-down", { action: label() })} onClick={() => clickMove(id, section, index() + 1)}>↓</button>
            <button type="button" class="settings-btn" aria-label={t(section === "bar" ? "toolbar-move-to-menu-label" : "toolbar-move-to-bar-label", { action: label() })}
              onClick={() => { const target = section === "bar" ? "menu" : "bar"; clickMove(id, target, lists()[target].length); }}>{t(section === "bar" ? "toolbar-move-to-menu" : "toolbar-move-to-bar")}</button>
          </li>;
        }}</For>
      </ul>
    </div>}</For>
    <button type="button" class="settings-btn" onClick={() => { if (!rowReorderClickSuppressed()) saveToolbarChoice(null); }}>{t("toolbar-reset")}</button>
  </section>;
}
