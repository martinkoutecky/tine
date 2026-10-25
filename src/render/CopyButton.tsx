import type { JSX } from "solid-js";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import { writeClipboardText } from "../clipboard";

// Shared raw-source copy affordance for inline code/links and fenced blocks.
export function CopyButton(props: { text: string; title: string; class?: string }): JSX.Element {
  const onCopy = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    void writeClipboardText(props.text)
      .then(() => pushToast(msg("render-copied-to-clipboard"), "success"))
      .catch(() => pushToast(msg("render-couldn-t-copy-to-clipboard"), "error"));
  };
  return (
    <button
      class={`copy-btn${props.class ? " " + props.class : ""}`}
      title={props.title}
      aria-label={props.title}
      onClick={onCopy}
    >
      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2">
        <rect x="9" y="9" width="11" height="11" rx="2" />
        <path d="M5 15V5a2 2 0 0 1 2-2h10" />
      </svg>
    </button>
  );
}
