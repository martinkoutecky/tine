import { For, Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { EmojiText } from "../render/emoji";
import { registerTransientLayer } from "../transientLayers";
import { pushToast } from "../ui";
import {
  activeWorkspaceId,
  createWorkspace,
  deleteWorkspace,
  renameWorkspace,
  switchWorkspace,
  workspaceDisplayName,
  workspaces,
  type Workspace,
} from "../workspaces";

type EditState = { kind: "new"; value: string } | { kind: "rename"; id: string; value: string };

export function WorkspaceSwitcher(): JSX.Element {
  let root: HTMLDivElement | undefined;
  let editInput: HTMLInputElement | undefined;
  const [hoverOpen, setHoverOpen] = createSignal(false);
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [edit, setEdit] = createSignal<EditState | null>(null);
  const active = createMemo(() => workspaces().find((workspace) => workspace.id === activeWorkspaceId()));

  const closeMenu = () => {
    setMenuOpen(false);
    setEdit(null);
  };

  createEffect(() => {
    if (!menuOpen()) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (root && target && !root.contains(target)) closeMenu();
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    const unregister = registerTransientLayer({
      id: "workspace-switcher",
      root: () => root ?? null,
      dismiss: () => { closeMenu(); return true; },
    });
    onCleanup(() => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      unregister();
    });
  });

  createEffect(() => {
    if (edit()) queueMicrotask(() => editInput?.focus());
  });

  const report = (operation: Promise<unknown>, message: string) => {
    void operation.catch((error) => pushToast(`${message}: ${String(error)}`, "error"));
  };

  const choose = (workspace: Workspace) => {
    setHoverOpen(false);
    closeMenu();
    report(switchWorkspace(workspace.id), "Couldn't switch workspace");
  };

  const submitEdit = () => {
    const state = edit();
    if (!state) return;
    const name = state.value.trim();
    if (!name) return;
    setEdit(null);
    if (state.kind === "new") report(createWorkspace(name), "Couldn't create workspace");
    else report(renameWorkspace(state.id, name), "Couldn't rename workspace");
  };

  const remove = async (workspace: Workspace) => {
    const name = workspaceDisplayName(workspace);
    if (!(await backend().confirm(`Delete workspace “${name}”?`, "Delete workspace"))) return;
    try {
      await deleteWorkspace(workspace.id);
    } catch (error) {
      pushToast(`Couldn't delete workspace: ${String(error)}`, "error");
    }
  };

  return (
    <div
      class="workspace-switcher"
      data-workspace-switcher
      ref={root}
      onMouseEnter={() => { if (!menuOpen() && workspaces().length) setHoverOpen(true); }}
      onMouseLeave={() => setHoverOpen(false)}
    >
      <button
        type="button"
        class="workspace-switcher-btn"
        aria-label={`Workspace: ${active() ? workspaceDisplayName(active()!) : "loading"}`}
        aria-haspopup="menu"
        aria-expanded={menuOpen()}
        disabled={!workspaces().length}
        onClick={() => {
          setHoverOpen(false);
          setMenuOpen((open) => !open);
          setEdit(null);
        }}
      >
        <span class="workspace-switcher-mark" aria-hidden="true">W</span>
        <span class="workspace-switcher-name">
          <EmojiText text={active() ? workspaceDisplayName(active()!) : "Workspace"} />
        </span>
        <span class="workspace-switcher-caret" aria-hidden="true">▾</span>
      </button>

      <Show when={hoverOpen() && !menuOpen()}>
        <div class="workspace-quick-menu" role="menu" aria-label="Quick switch workspace">
          <For each={workspaces()}>
            {(workspace) => (
              <button
                type="button"
                role="menuitem"
                class="workspace-quick-row"
                classList={{ active: workspace.id === activeWorkspaceId() }}
                onClick={() => choose(workspace)}
              >
                <EmojiText text={workspaceDisplayName(workspace)} />
              </button>
            )}
          </For>
        </div>
      </Show>

      <Show when={menuOpen()}>
        <div class="workspace-menu" role="menu" aria-label="Workspaces">
          <For each={workspaces()}>
            {(workspace) => (
              <div class="workspace-menu-row" classList={{ active: workspace.id === activeWorkspaceId() }}>
                <button type="button" role="menuitem" class="workspace-menu-name" onClick={() => choose(workspace)}>
                  <EmojiText text={workspaceDisplayName(workspace)} />
                </button>
                <button
                  type="button"
                  class="workspace-menu-action"
                  aria-label={`Rename ${workspaceDisplayName(workspace)}`}
                  title="Rename"
                  onClick={() => setEdit({ kind: "rename", id: workspace.id, value: workspaceDisplayName(workspace) })}
                >
                  Rename
                </button>
                <button
                  type="button"
                  class="workspace-menu-action danger"
                  aria-label={`Delete ${workspaceDisplayName(workspace)}`}
                  title="Delete"
                  onClick={() => void remove(workspace)}
                >
                  Delete
                </button>
              </div>
            )}
          </For>
          <div class="ctx-sep" />
          <button
            type="button"
            role="menuitem"
            class="workspace-new-btn"
            onClick={() => setEdit({ kind: "new", value: "" })}
          >
            + New workspace
          </button>
          <Show when={edit()} keyed>
            {(state) => (
              <form
                class="workspace-edit-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  submitEdit();
                }}
              >
                <input
                  ref={editInput}
                  class="workspace-edit-input"
                  aria-label={state.kind === "new" ? "New workspace name" : "Workspace name"}
                  value={state.value}
                  maxlength={80}
                  onInput={(event) => setEdit({ ...state, value: event.currentTarget.value })}
                  onKeyDown={(event) => { if (event.key === "Escape") setEdit(null); }}
                />
                <button type="submit" disabled={!state.value.trim()}>{state.kind === "new" ? "Create" : "Save"}</button>
              </form>
            )}
          </Show>
        </div>
      </Show>
    </div>
  );
}
