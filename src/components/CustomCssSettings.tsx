import { Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { customCssDisabled, setCustomCssDisabled } from "../customCss";
import { bindingOwner, ownedWhen, readOwned, writeOwned } from "../owned";
import { platformKind } from "../platform";
import { pushToast } from "../toasts";
import { openDevtools } from "../ui";
import { Field, Toggle } from "./settingsField";

/** Settings > Appearance > Custom CSS (GH #610): the three affordances that
 *  make `logseq/custom.css` discoverable and recoverable.
 *  - Edit custom.css (desktop): create the file through the store's audited
 *    create when missing, then open it in the system editor. Android has no
 *    editor hand-off, so it keeps the file path hint instead.
 *  - Live reload needs no control: the watcher re-applies outside edits.
 *  - Disable custom CSS: a session-only safe mode (never persisted).
 *  - Developer tools (desktop): the inspector, to find what styles an element. */
export function CustomCssSettings(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  const [desktop, setDesktop] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  onMount(() => {
    void readOwned(ownedWhen(() => alive), platformKind()).then((result) => {
      if (result.kind === "current") setDesktop(result.value === "desktop");
    });
  });
  const edit = async () => {
    setBusy(true);
    try {
      const result = await writeOwned(bindingOwner(), backend().editCustomCss());
      if (alive && result.kind === "current") pushToast("Opened logseq/custom.css in your editor. Saving it applies the changes here at once.", "info");
    } catch (error) {
      pushToast(`Could not open logseq/custom.css: ${String(error)}`, "error");
    } finally {
      if (alive) setBusy(false);
    }
  };
  return (
    <>
      <div class="settings-section">Custom CSS</div>
      <Show when={desktop()}>
        <Field
          label="Edit custom.css"
          hint={<>Opens <code>logseq/custom.css</code> in your editor, creating a starter file if the graph has none. Changes apply here as soon as you save; no restart. See the Guide page "Customize Tine's look" for the supported <code>--tine-*</code> tokens and recipes.</>}
        >
          <button class="settings-btn" disabled={busy()} onClick={() => void edit()}>
            {busy() ? "Opening…" : "Edit custom.css"}
          </button>
        </Field>
      </Show>
      <Show when={!desktop()}>
        <div class="settings-hint theme-gallery-hint">
          Edit <code>logseq/custom.css</code> in your graph folder with any editor; Tine re-applies it when the file changes. See the Guide page "Customize Tine's look".
        </div>
      </Show>
      <Field
        label="Disable custom CSS"
        hint="Safe mode for this session only: ignores logseq/custom.css until you switch it back on or restart Tine. Use it if a stylesheet made something unreadable."
      >
        <Toggle on={customCssDisabled()} onClick={() => setCustomCssDisabled(!customCssDisabled())} />
      </Field>
      <Show when={desktop()}>
        <Field
          label="Developer tools"
          hint={<>Opens the inspector to see which rules style an element (also <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd>).</>}
        >
          <button class="settings-btn" onClick={() => openDevtools()}>Open developer tools</button>
        </Field>
      </Show>
    </>
  );
}
