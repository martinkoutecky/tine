import { For, Show, createSignal, type JSX } from "solid-js";
import {
  contextMenu,
  closeContextMenu,
  zoomInto,
  openBlockInSidebar,
  openPageInSidebar,
  isFavorite,
  toggleFavorite,
  pushToast,
  graphMeta,
  setJournalTemplate,
  openPageProps,
  openExportModal,
} from "../ui";
import { openPage, openPageInNewTab, openJournals, route } from "../router";
import { backend } from "../backend";
import { carryDay } from "../carry";
import { journalTitle } from "../journal";
import {
  doc,
  ensureBlockId,
  persistentBlockRef,
  blockSubtreeMarkdown,
  deleteBlock,
  setBlockProperty,
  toggleBlockProperty,
  blockProperty,
  setHeading,
  setCollapsedDeep,
  dtoSubtreeMarkdown,
  flushAll,
  deletePage,
  selectedIds,
} from "../store";

// Copy a block reference/embed — but only after the block's id:: is durably on
// disk. ensureBlockId returns null if the save couldn't land (conflict/error), in
// which case we must NOT copy a ref that would dangle after a restart.
async function copyBlockRef(id: string, fmt: (uuid: string) => string, okMsg: string) {
  const uuid = await ensureBlockId(id);
  if (!uuid) {
    pushToast("Couldn't save the block id — reference not copied (resolve the conflict first).", "error");
    return;
  }
  await backend().writeText(fmt(uuid));
  pushToast(okMsg, "success");
}

// Block background colors, matching Logseq's built-in set.
const COLORS = ["yellow", "red", "pink", "green", "blue", "purple", "gray"];
const COLOR_BG: Record<string, string> = {
  yellow: "#fbe69e",
  red: "#f5a3a3",
  pink: "#f3b0d4",
  green: "#a6e3b4",
  blue: "#a8c9f0",
  purple: "#cdb4ee",
  gray: "#d3d6da",
};

// Right-click context menu. Universal over its target: a block (full editing
// menu — colors, headings, open/copy/cut, collapse, numbered list) or a page
// reference (open / open in sidebar / new tab / copy ref). The target is
// whatever you right-clicked, so right-clicking a [[page]] acts on the page,
// not the block that contains it.
export function ContextMenu(): JSX.Element {
  const close = () => closeContextMenu();

  return (
    <Show when={contextMenu()}>
      {(m) => (
        <div class="ctx-overlay" onClick={close} onContextMenu={(e) => { e.preventDefault(); close(); }}>
          <div
            class="ctx-menu"
            style={{ left: `${m().x}px`, top: `${m().y}px` }}
            onClick={(e) => e.stopPropagation()}
          >
            <Show
              when={m().kind === "block"}
              fallback={
                <PageMenu
                  name={(m() as { name: string }).name}
                  pageKind={(m() as { pageKind: "journal" | "page" }).pageKind}
                  x={m().x}
                  y={m().y}
                  close={close}
                />
              }
            >
              <BlockMenu id={(m() as { blockId: string }).blockId} close={close} />
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}

function BlockMenu(props: { id: string; close: () => void }): JSX.Element {
  return (
    <>
      {/* Color row */}
      <div class="ctx-row ctx-colors">
        <button
          class="ctx-color ctx-color-none"
          title="No background"
          onClick={() => { setBlockProperty(props.id, "background-color", null); props.close(); }}
        >
          ✕
        </button>
        <For each={COLORS}>
          {(c) => (
            <button
              class="ctx-color"
              title={c}
              style={{ background: COLOR_BG[c] }}
              onClick={() => { toggleBlockProperty(props.id, "background-color", c); props.close(); }}
            />
          )}
        </For>
      </div>

      {/* Heading row */}
      <div class="ctx-row ctx-headings">
        <For each={[1, 2, 3, 4, 5, 6]}>
          {(h) => (
            <button class="ctx-h" title={`Heading ${h}`} onClick={() => { setHeading(props.id, h); props.close(); }}>
              H{h}
            </button>
          )}
        </For>
        <button class="ctx-h" title="Remove heading" onClick={() => { setHeading(props.id, null); props.close(); }}>
          ⌫
        </button>
      </div>

      <div class="ctx-sep" />

      <For each={blockActions(props.id)}>
        {(it) => (
          <div
            class="ctx-item"
            classList={{ danger: !!it.danger }}
            onClick={() => { it.run(); props.close(); }}
          >
            {it.label}
          </div>
        )}
      </For>

      <div class="ctx-sep" />
      <MakeTemplate id={props.id} close={props.close} />
    </>
  );
}

// "Make a template" — mirrors OG Logseq: a context-menu action that expands into
// an inline name field (+ an "Include parent block" toggle when the block has
// children), and on submit marks the block with `template:: <name>`. The block
// stays where it is; that property is the template. Insert it later via `/<name>`.
function MakeTemplate(props: { id: string; close: () => void }): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  const [name, setName] = createSignal("");
  // OG default: the toggle starts ON when the block has children — the named
  // block is inserted together with its children. Off → `template-including-parent::
  // false` (only the children are inserted; the block is just the template's label).
  const [includeParent, setIncludeParent] = createSignal(true);
  const hasChildren = () => (doc.byId[props.id]?.children.length ?? 0) > 0;

  const submit = async () => {
    const title = name().trim();
    if (!title) return;
    const existing = await backend().listTemplates().catch(() => []);
    if (existing.some((t) => t.name.toLowerCase() === title.toLowerCase())) {
      pushToast(`A template named “${title}” already exists.`, "error");
      return;
    }
    setBlockProperty(props.id, "template", title);
    if (hasChildren() && !includeParent()) {
      setBlockProperty(props.id, "template-including-parent", "false");
    }
    pushToast(`Template “${title}” created.`, "success");
    props.close();
  };

  return (
    <Show
      when={editing()}
      fallback={
        <div class="ctx-item" onClick={(e) => { e.stopPropagation(); setEditing(true); }}>
          Make a template…
        </div>
      }
    >
      <div class="ctx-template-form">
        <input
          class="ctx-template-name"
          placeholder="Template name"
          autofocus
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); void submit(); }
            else if (e.key === "Escape") { e.preventDefault(); setEditing(false); }
          }}
        />
        <Show when={hasChildren()}>
          <label class="ctx-template-toggle">
            <input
              type="checkbox"
              checked={includeParent()}
              onChange={(e) => setIncludeParent(e.currentTarget.checked)}
            />
            Include parent block
          </label>
        </Show>
        <button class="ctx-template-submit" onClick={() => void submit()}>
          Create template
        </button>
        <div class="ctx-template-hint">
          Tip: in the template, <code>{"<% today %>"}</code>, <code>{"<% current page %>"}</code>,{" "}
          <code>{"<% date: +3d %>"}</code> expand when it's inserted (or via <code>/Template var</code>).
        </div>
      </div>
    </Show>
  );
}

function PageMenu(props: {
  name: string;
  pageKind: "journal" | "page";
  x: number;
  y: number;
  close: () => void;
}): JSX.Element {
  const fav = () => isFavorite(props.name);
  const rename = () => {
    const next = window.prompt("Rename page to:", props.name)?.trim();
    if (next && next !== props.name) {
      // Persist ALL unsaved edits first — the rename transaction reads every
      // referencing page from disk to rewrite its `[[refs]]`, so a dirty edit on
      // ANY page (not just the renamed one) would be read stale and lost. Abort if
      // anything couldn't be saved.
      void flushAll()
        .then((ok) => {
          if (!ok) {
            pushToast("Couldn't save pending edits — resolve the conflict before renaming.", "error");
            return;
          }
          return backend()
            .renamePage(props.name, next)
            .then(() => {
              openPage(next, props.pageKind);
              pushToast(`Renamed to “${next}”`, "success");
            });
        })
        .catch(() => pushToast("Rename failed", "error"));
    }
  };
  const remove = () => {
    if (!window.confirm(`Delete page "${props.name}"? This cannot be undone.`)) return;
    // Route through the store (not backend directly) so it tombstones the page and
    // cancels any pending save — otherwise a just-typed, never-saved page could be
    // recreated by a queued save right after we delete it.
    void deletePage(props.name, props.pageKind)
      .then((ok) => {
        if (!ok) {
          pushToast("Delete failed", "error");
          return;
        }
        const r = route();
        if (r.kind === "page" && r.name === props.name) openJournals();
        pushToast(`Deleted “${props.name}”`, "success");
      })
      .catch(() => pushToast("Delete failed", "error"));
  };
  const items: { label: string; run: () => void; danger?: boolean }[] = [
    { label: "Open", run: () => openPage(props.name, props.pageKind) },
    { label: "Open in sidebar", run: () => openPageInSidebar(props.name, props.pageKind) },
    { label: "Open in new tab", run: () => openPageInNewTab(props.name, props.pageKind) },
    { label: fav() ? "Remove from favorites" : "Add to favorites", run: () => toggleFavorite(props.name, props.pageKind) },
    { label: "Copy page ref", run: () => { void backend().writeText(`[[${props.name}]]`); pushToast("Copied page ref", "success"); } },
    {
      label: "Copy page as Markdown",
      run: () =>
        void backend()
          .getPage(props.name, props.pageKind)
          .then((p) => {
            if (p) backend().writeText(p.blocks.map((b) => dtoSubtreeMarkdown(b)).join("\n"));
            pushToast("Copied page as Markdown", "success");
          }),
    },
    { label: "Page properties…", run: () => openPageProps(props.name, props.x, props.y) },
    // Carry a past day's unfinished tasks to today (journal days only, not today).
    ...(props.pageKind === "journal" && props.name !== journalTitle(new Date())
      ? [{ label: "Carry unfinished tasks → today", run: () => void carryDay(props.name) }]
      : []),
    ...(props.pageKind === "page"
      ? [
          { label: "Rename page", run: rename },
          { label: "Delete page", run: remove, danger: true },
        ]
      : []),
  ];
  return (
    <For each={items}>
      {(it) => (
        <div class="ctx-item" classList={{ danger: !!it.danger }} onClick={() => { it.run(); props.close(); }}>
          {it.label}
        </div>
      )}
    </For>
  );
}

function blockActions(id: string): { label: string; run: () => void; danger?: boolean }[] {
  const numbered = blockProperty(id, "logseq.order-list-type") === "number";
  // If this block is itself a template (`template:: name`), offer to set it as the
  // new-journal default (or clear it if it already is) — right where templates live.
  const tmplName = blockProperty(id, "template");
  const isJournalTmpl = !!tmplName && graphMeta()?.default_journal_template === tmplName;
  return [
    { label: "Open in sidebar", run: () => openBlockInSidebar(persistentBlockRef(id)) },
    { label: "Zoom into block", run: () => zoomInto(id) },
    { label: "Copy block ref", run: () => void copyBlockRef(id, (u) => `((${u}))`, "Copied block ref") },
    { label: "Copy block embed", run: () => void copyBlockRef(id, (u) => `{{embed ((${u}))}}`, "Copied block embed") },
    { label: "Copy block", run: () => { void backend().writeText(blockSubtreeMarkdown(id)); pushToast("Copied block", "success"); } },
    // Open the export modal for the whole selection (if this block is part of a
    // multi-selection) or just this block's subtree — preview + indent/remove opts.
    {
      label: "Copy / export as…",
      run: () => {
        const sel = selectedIds();
        openExportModal(sel.length > 1 && sel.includes(id) ? sel : [id]);
      },
    },
    {
      label: "Cut block",
      run: () => {
        void backend().writeText(blockSubtreeMarkdown(id));
        deleteBlock(id);
      },
    },
    {
      label: numbered ? "Remove numbered list" : "Numbered list",
      run: () => toggleBlockProperty(id, "logseq.order-list-type", "number"),
    },
    { label: "Collapse all", run: () => setCollapsedDeep(id, true) },
    { label: "Expand all", run: () => setCollapsedDeep(id, false) },
    ...(tmplName
      ? [
          {
            label: isJournalTmpl ? "✓ Used for new journals" : "Use for new journals",
            run: () => setJournalTemplate(isJournalTmpl ? null : tmplName),
          },
        ]
      : []),
    { label: "Delete block", run: () => deleteBlock(id), danger: true },
  ];
}
