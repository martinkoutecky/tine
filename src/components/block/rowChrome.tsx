import { t } from "../../i18n";
import { createMemo, type JSX } from "solid-js";
import { collapsibleDescendantIds, node as docNode, setCollapsedDescendants } from "../../document";
import { bulkCollapseTarget } from "../../collapseToggle";
import type { CollapseSurfaceApi } from "../Block";
import { acceptsPluginColor, type PluginSettingValue } from "../../plugins/settings";

// Row chrome of `Block` that only some rows need, split out so `Block.tsx` (over
// the 1,500-line ceiling) does not grow (og I3, master 0350c00b6 / ce9a796fb).

export interface ThreadLineDecoration {
  enabled: boolean;
  active: boolean;
  standard: boolean;
  thickness?: number;
  color?: string;
  levels?: boolean;
}
export const NO_THREAD_LINES: ThreadLineDecoration = { enabled: false, active: false, standard: false };

export function threadLineSettings(enabled: boolean, setting: (key: string) => PluginSettingValue | undefined): ThreadLineDecoration {
  const thickness = setting("thickness");
  const color = setting("color");
  return {
    enabled,
    active: setting("display") === "active",
    standard: setting("intensity") === "standard",
    thickness: typeof thickness === "number" && Number.isFinite(thickness) && thickness >= 0 && thickness <= 10 ? thickness : 1,
    color: acceptsPluginColor(color) ? color : "",
    levels: setting("color-by-level") === true,
  };
}

export function rowDecorationStyle(threadLines: ThreadLineDecoration, depth = 0): JSX.CSSProperties {
  if (!threadLines.enabled) return {};
  return {
    "--thread-width": `${threadLines.thickness ?? 1}px`,
    "--thread-color": threadLines.color || "var(--ls-link-text-color)",
    "--thread-active-color": threadLines.levels
      ? ["var(--ls-link-text-color)", "var(--ls-tag-text-color, var(--text-primary))", "color-mix(in srgb, var(--ls-link-text-color) 50%, var(--ls-tag-text-color))", "color-mix(in srgb, var(--ls-tag-text-color) 65%, var(--text-primary))"][depth % 4]
      : "var(--thread-color)",
  };
}

/** The plugin thread-lines classes of the `.ls-block` row. */
export function rowDecorationClasses(threadLines: ThreadLineDecoration): Record<string, boolean> {
  return {
    "plugin-thread-lines": threadLines.enabled,
    "plugin-thread-lines-active": threadLines.enabled && threadLines.active,
    "plugin-thread-lines-standard": threadLines.enabled && threadLines.standard,
    "plugin-thread-lines-hidden": threadLines.enabled && threadLines.thickness === 0,
    "plugin-thread-lines-levels": threadLines.enabled && threadLines.levels === true,
  };
}

/** The children container's "collapse/expand every descendant" left border. Its
 *  two derivations (the descendant list and whether any of them is folded) live
 *  HERE rather than in `Block`, so a leaf never allocates them and the subtree
 *  walk stays memoized only for containers that render this control. */
export function CollapseAllBorder(props: { id: string; readOnly: boolean; surface: CollapseSurfaceApi | null }): JSX.Element {
  const collapseSurface = props.surface;
  const collapsibleDescendants = createMemo(() => collapsibleDescendantIds(props.id));
  const hasCollapsedDescendant = createMemo(() =>
    !bulkCollapseTarget(collapsibleDescendants(), (id) => {
      const descendant = docNode(id);
      return descendant
        ? collapseSurface?.collapsed(id, descendant.collapsed) ?? descendant.collapsed
        : false;
    })
  );
  const toggleCollapsedDescendants = () => {
    const ids = collapsibleDescendants();
    if (!ids.length || (props.readOnly && !collapseSurface)) return;
    // OG semantics: any folded descendant means “expand all”; only a completely
    // open subtree means “collapse all”. The guide parent itself stays open.
    const next = !hasCollapsedDescendant();
    if (collapseSurface) collapseSurface.setMany(ids, next);
    else setCollapsedDescendants(props.id, next);
  };
  return (
    <button
      type="button"
      class="block-children-left-border"
      aria-label={hasCollapsedDescendant() ? t("row-chrome-expand-all-descendants") : t("row-chrome-collapse-all-descendants")}
      aria-expanded={!hasCollapsedDescendant()}
      disabled={collapsibleDescendants().length === 0 || (props.readOnly && !collapseSurface)}
      title={hasCollapsedDescendant() ? t("row-chrome-expand-all-descendants") : t("row-chrome-collapse-all-descendants")}
      onClick={(event) => {
        event.stopPropagation();
        toggleCollapsedDescendants();
      }}
    />
  );
}
