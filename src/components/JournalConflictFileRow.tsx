import { Show, createEffect, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, readOwned } from "../owned";
import { registerTransientLayer } from "../transientLayers";
import type { JournalFile } from "../types";
import { readOr } from "../resourceRead";

// One file in a duplicate-day conflict. Click the name to reveal its full
// contents; the action buttons let you reach and reconcile it (#21): Open
// navigates to THIS specific file (editable, saves back to itself), Merge folds a
// stray into the canonical day, Rename rescues it as a normal page, Trash removes
// the redundant one (recoverable).
/** Exported because the in-page conflict panel renders the same rows (master
 *  9dc54e4a7): one renderer, not two that drift. `parentLayerId` attaches the
 *  transient layers (content preview, rename box) to the hosting surface. */
export function ConflictFileRow(props: {
  file: JournalFile;
  onOpen: () => void;
  onMerge?: () => void;
  onRename: (newName: string) => void;
  onTrash: () => void;
  parentLayerId?: string;
}): JSX.Element {
  const rowLayerId = `journal-conflict-${props.file.path}`;
  let renameRoot: HTMLDivElement | undefined;
  let contentRoot: HTMLPreElement | undefined;
  const [open, setOpen] = createSignal(false);
  const [renaming, setRenaming] = createSignal(false);
  const [newName, setNewName] = createSignal("");
  const [contentResource] = createResource(
    () => (open() ? props.file.name : null),
    // Owned by the graph that listed the file: a switch empties the list, and
    // a read landing after it shows nothing from the other graph.
    async (name) => {
      if (!name) return "";
      try {
        const read = await readOwned(graphOwner(), backend().readJournalFile(name));
        return read.kind === "current" ? read.value : "";
      } catch (e) {
        return `(couldn’t read: ${String(e)})`;
      }
    }
  );
  // The fetcher already turns a read failure into readable text; this covers
  // the read itself, and says the same true thing rather than "(empty file)".
  const content = () => readOr(contentResource, "(couldn’t read this file)", "journal conflict file");
  const submitRename = () => {
    const n = newName().trim();
    if (n) props.onRename(n);
    setRenaming(false);
    setNewName("");
  };
  createEffect(() => {
    if (!open()) return;
    const unregister = registerTransientLayer({
      id: `${rowLayerId}-content`,
      parentId: props.parentLayerId ?? "settings",
      root: () => contentRoot ?? null,
      dismiss: () => { setOpen(false); return true; },
    });
    onCleanup(unregister);
  });
  createEffect(() => {
    if (!renaming()) return;
    const unregister = registerTransientLayer({
      id: `${rowLayerId}-rename`,
      parentId: props.parentLayerId ?? "settings",
      root: () => renameRoot ?? null,
      dismiss: () => { setRenaming(false); setNewName(""); return true; },
    });
    onCleanup(unregister);
  });
  return (
    <>
      <div class="journal-conflict-row" data-journal-conflict={props.file.path}>
        <button class="settings-asset-name mono" title="Show this file's contents" onClick={() => setOpen(!open())}>
          {open() ? "▾ " : "▸ "}
          {props.file.name}
          <Show when={props.file.canonical}>
            <span class="journal-conflict-keep"> · canonical</span>
          </Show>
        </button>
        <span class="journal-conflict-actions">
          <button class="settings-btn" title="Open this exact file (editable)" onClick={props.onOpen}>
            Open
          </button>
          <Show when={props.onMerge}>
            <button class="settings-btn" title="Append this file's blocks to the canonical day, then trash it" onClick={props.onMerge}>
              Merge
            </button>
          </Show>
          <button class="settings-btn" title="Move this file to a uniquely-named page" onClick={() => { setRenaming(true); setNewName(""); }}>
            Rename…
          </button>
          <button class="settings-btn settings-btn-danger" onClick={props.onTrash}>
            Trash
          </button>
        </span>
      </div>
      <div class="journal-conflict-preview">
        {props.file.preview_error ? `Couldn't read this file: ${props.file.preview_error}` : props.file.preview}
      </div>
      <Show when={renaming()}>
        <div ref={renameRoot} class="journal-conflict-rename">
          <input
            class="settings-input"
            placeholder="New page name"
            value={newName()}
            onInput={(e) => setNewName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.isComposing || e.keyCode === 229) return;
              if (e.key === "Enter") submitRename();
              else if (e.key === "Escape") setRenaming(false);
            }}
          />
          <button class="settings-btn" onClick={submitRename}>Save</button>
          <button class="settings-btn" onClick={() => setRenaming(false)}>Cancel</button>
        </div>
      </Show>
      <Show when={open()}>
        <pre ref={contentRoot} class="journal-conflict-content">{contentResource.loading ? "…" : content() || "(empty file)"}</pre>
      </Show>
    </>
  );
}
