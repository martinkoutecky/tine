import { onCleanup, onMount, type JSX } from "solid-js";
import { registerTransientLayer, useModal } from "../transientLayers";
import { documentOf } from "../windowRealm";
import "./RenameAliasDialog.css";

/** The rename dialogs' frame (alias dialog, day move, contested name): a modal
 *  that Escape, dismissal and unmounting answer with `dismiss`, keeps Tab
 *  inside, and focuses its first `[data-choice]` button. O(buttons) per Tab. */
export function RenameDialogFrame(props: { id: string; label: string; dismiss: () => void; children: JSX.Element }) {
  useModal();
  let root!: HTMLElement;
  onCleanup(() => props.dismiss());
  onMount(() => {
    onCleanup(registerTransientLayer({ id: props.id, root: () => root,
      dismiss: () => { props.dismiss(); return true; } }));
    root.querySelector<HTMLButtonElement>("button[data-choice]")?.focus();
  });
  return (
    <div class="modal-overlay tine-rename-alias-overlay" onKeyDown={(event) => {
      if (event.key === "Escape") props.dismiss();
      if (event.key === "Tab") {
        const buttons = [...root.querySelectorAll<HTMLButtonElement>("button")];
        const first = buttons[0], last = buttons.at(-1);
        if (event.shiftKey && documentOf(event).activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && documentOf(event).activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <section ref={root} class="tine-rename-alias-dialog" role="dialog" aria-modal="true" aria-label={props.label}>
        {props.children}
      </section>
    </div>
  );
}
