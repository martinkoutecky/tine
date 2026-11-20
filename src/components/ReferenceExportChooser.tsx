import { t } from "../i18n";
import { DialogActions } from "./DialogActions";
import { For, Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { sourceFormatForPage } from "../document";
import { referenceGroupKey } from "../referenceGroups";
import { visibleBody } from "../render/block";
import { EmojiText } from "../render/emoji";
import { openReferenceResultsCopy } from "./queryResultsCopy";
import type { BlockDto, RefGroup } from "../types";
import { registerTransientLayer } from "../transientLayers";

/** Choose from a stable snapshot of the filtered reference rows. Opening
 * costs O(reference rows). Continuing reads only the chosen source pages. */
export function ReferenceExportChooser(props: {
  subject: string;
  groups: RefGroup[];
  onClose: () => void;
}): JSX.Element {
  let root: HTMLDivElement | undefined;
  const groups = props.groups;
  let alive = true;
  onCleanup(() => { alive = false; });
  createEffect(() => {
    const unregister = registerTransientLayer({
      id: `reference-export:${props.subject}`,
      root: () => root ?? null,
      dismiss: () => { props.onClose(); return true; },
    });
    onCleanup(unregister);
  });
  const entryKey = (group: RefGroup, block: BlockDto) => `${group.kind}\0${referenceGroupKey(group)}\0${block.id}`;
  const allKeys = () => groups.flatMap((group) => group.blocks.map((block) => entryKey(group, block)));
  const [selected, setSelected] = createSignal(new Set(allKeys()));
  const [opening, setOpening] = createSignal(false);
  const count = () => selected().size;
  const total = count();
  const toggle = (key: string, checked: boolean) => setSelected((current) => {
    const next = new Set(current);
    if (checked) next.add(key); else next.delete(key);
    return next;
  });
  const exportSelection = async () => {
    if (opening() || !count()) return;
    const keys = selected();
    const chosen = groups.flatMap(group => {
      const blocks = group.blocks.filter(block => keys.has(entryKey(group, block)));
      return blocks.length ? [{ ...group, blocks }] : [];
    });
    setOpening(true);
    const opened = await openReferenceResultsCopy(chosen, () => alive);
    if (!alive) return;
    if (opened) props.onClose();
    else setOpening(false);
  };

  return (
    <div class="modal-overlay" onClick={props.onClose}>
      <div ref={root} class="export-modal ref-export-chooser" role="dialog"
        aria-label={t("reference-export-chooser-copy-export", { subject: String(props.subject) })} onClick={(event) => event.stopPropagation()}>
        <div class="export-head">{t("reference-export-chooser-copy-export", { subject: props.subject })}<span class="export-count">{t("reference-export-chooser-count", { count: String(count()), total: String(total) })}</span></div>
        <div class="ref-export-actions" role="group" aria-label={t("reference-export-chooser-reference-selection")}>
          <button type="button" disabled={opening() || !total} onClick={() => setSelected(new Set(allKeys()))}>{t("reference-export-chooser-all")}</button>
          <button type="button" disabled={opening() || !count()} onClick={() => setSelected(new Set())}>{t("reference-export-chooser-none")}</button>
        </div>
        <div class="ref-export-list" role="group" aria-label={t("reference-export-chooser-entries", { subject: String(props.subject) })}>
          <For each={groups}>{(group) => {
            const format = () => sourceFormatForPage(group.page, group.kind, group.path);
            const title = (block: BlockDto) => {
              const sourceFormat = format();
              return sourceFormat ? visibleBody(block.raw, sourceFormat)[0] ?? "" : block.raw;
            };
            return (
            <div class="ref-export-group">
              <div class="ref-export-group-name"><EmojiText text={group.page} /></div>
              <For each={group.blocks}>{(block) => {
                const key = entryKey(group, block);
                return <label class="ref-export-row">
                  <input type="checkbox" checked={selected().has(key)} disabled={opening()}
                    onChange={(event) => toggle(key, event.currentTarget.checked)} />
                  <span class="ref-export-text"><EmojiText text={title(block)} /></span>
                </label>;
              }}</For>
            </div>
            );
          }}</For>
          <Show when={total === 0}><div class="ref-export-empty">{t("reference-export-chooser-no-entries-in-this-section")}</div></Show>
        </div>
        <DialogActions
          cancel={<button class="export-btn-secondary" onClick={props.onClose}>{t("reference-export-chooser-cancel")}</button>}
          primary={<button class="export-btn-primary" disabled={!count() || opening()} onClick={exportSelection}>
            {t(opening() ? "export-modal-resolving" : "reference-export-chooser-open")}
          </button>} />
      </div>
    </div>
  );
}
