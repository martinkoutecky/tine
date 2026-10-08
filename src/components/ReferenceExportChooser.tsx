import { For, Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { formatForPage } from "../document";
import { openExportNodesModal } from "../ui";
import { visibleBody } from "../render/block";
import { EmojiText } from "../render/emoji";
import { blockDtosToExportNodes } from "./ExportModal";
import type { ExportNode } from "../editor/exportText";
import type { BlockDto, RefGroup } from "../types";
import { registerTransientLayer } from "../transientLayers";

/** Select a stable snapshot of the visible references and hand their DTOs to
 * the shared export dialog. Work costs O(visible reference blocks); the graph
 * is read only and a refresh while open cannot change the selected rows. */
export function ReferenceExportChooser(props: {
  subject: string;
  groups: RefGroup[];
  onClose: () => void;
}): JSX.Element {
  let root: HTMLDivElement | undefined;
  const groups = props.groups;
  createEffect(() => {
    const unregister = registerTransientLayer({
      id: `reference-export:${props.subject}`,
      root: () => root ?? null,
      dismiss: () => { props.onClose(); return true; },
    });
    onCleanup(unregister);
  });
  const entryKey = (group: RefGroup, block: BlockDto) => `${group.kind}\0${group.page}\0${block.id}`;
  const allKeys = () => groups.flatMap((group) => group.blocks.map((block) => entryKey(group, block)));
  const [selected, setSelected] = createSignal(new Set(allKeys()));
  const count = () => selected().size;
  const total = () => allKeys().length;
  const toggle = (key: string, checked: boolean) => setSelected((current) => {
    const next = new Set(current);
    if (checked) next.add(key); else next.delete(key);
    return next;
  });
  const exportSelection = () => {
    const nodes = groups.flatMap((group): ExportNode[] => {
      const chosen = group.blocks.filter((block) => selected().has(entryKey(group, block)));
      if (!chosen.length) return [];
      const format = formatForPage(group.page);
      return [{ raw: group.page, format, children: blockDtosToExportNodes(chosen, format) }];
    });
    if (!nodes.length) return;
    openExportNodesModal(nodes, count());
    props.onClose();
  };

  return (
    <div class="modal-overlay" onClick={props.onClose}>
      <div ref={root} class="export-modal ref-export-chooser" role="dialog"
        aria-label={`Copy / export ${props.subject}`} onClick={(event) => event.stopPropagation()}>
        <div class="export-head">Copy / export {props.subject}<span class="export-count">{count()} of {total()}</span></div>
        <div class="ref-export-actions" role="group" aria-label="Reference selection">
          <button type="button" disabled={!total()} onClick={() => setSelected(new Set(allKeys()))}>All</button>
          <button type="button" disabled={!count()} onClick={() => setSelected(new Set())}>None</button>
        </div>
        <div class="ref-export-list" role="group" aria-label={`${props.subject} entries`}>
          <For each={groups}>{(group) => (
            <div class="ref-export-group">
              <div class="ref-export-group-name"><EmojiText text={group.page} /></div>
              <For each={group.blocks}>{(block) => {
                const key = entryKey(group, block);
                return <label class="ref-export-row">
                  <input type="checkbox" checked={selected().has(key)}
                    onChange={(event) => toggle(key, event.currentTarget.checked)} />
                  <span class="ref-export-text"><EmojiText text={visibleBody(block.raw)[0] ?? ""} /></span>
                </label>;
              }}</For>
            </div>
          )}</For>
          <Show when={total() === 0}><div class="ref-export-empty">No entries in this section.</div></Show>
        </div>
        <div class="export-foot">
          <button class="export-btn-secondary" onClick={props.onClose}>Cancel</button>
          <button class="export-btn-primary" disabled={!count()} onClick={exportSelection}>Copy / export…</button>
        </div>
      </div>
    </div>
  );
}
