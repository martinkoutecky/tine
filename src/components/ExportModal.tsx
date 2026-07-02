import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { exportModal, closeExportModal, pushToast, typographyMode } from "../ui";
import { exportNodesFor } from "../store";
import { backend } from "../backend";
import {
  exportOutline,
  DEFAULT_EXPORT_OPTIONS,
  type ExportContent,
  type ExportOptions,
  type IndentStyle,
} from "../editor/exportText";

const STORE_KEY = "tine.exportOptions";

// Persist the last-used options so the modal opens the way you left it (the
// indent style especially — most people pick one and keep it).
function loadOptions(): ExportOptions {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw) return { ...DEFAULT_EXPORT_OPTIONS, ...JSON.parse(raw) };
  } catch {
    /* ignore malformed/missing */
  }
  return { ...DEFAULT_EXPORT_OPTIONS };
}
function saveOptions(o: ExportOptions): void {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(o));
  } catch {
    /* storage unavailable — non-fatal */
  }
}

const CONTENT_STYLES: { value: ExportContent; label: string; hint: string }[] = [
  { value: "rendered", label: "Rendered", hint: "the text as displayed — glyphs (→ –), no markup markers" },
  { value: "source", label: "Source", hint: "the raw Markdown/Org text" },
];

const INDENT_STYLES: { value: IndentStyle; label: string; hint: string }[] = [
  { value: "dashes", label: "Dashes", hint: "Logseq outline (- bullets)" },
  { value: "spaces", label: "Spaces", hint: "indent, no bullets" },
  { value: "no-indent", label: "No indent", hint: "flat, no bullets" },
];

// `sourceOnly` toggles are moot in rendered mode (the markers are already gone).
const TOGGLES: { key: keyof ExportOptions; label: string; sourceOnly?: boolean }[] = [
  { key: "stripLinks", label: "[[links]] → text" },
  { key: "removeEmphasis", label: "Remove emphasis", sourceOnly: true },
  { key: "removeTags", label: "Remove #tags" },
  { key: "removeProperties", label: "Remove properties" },
  { key: "newlineAfterBlock", label: "Newline after block" },
];

// "Copy / Export" modal — live-preview text export of a block subtree or a
// multi-block selection, with indent-style + remove options (mirrors OG Logseq's
// export dialog). Read-only preview; Copy writes to the clipboard.
export function ExportModal(): JSX.Element {
  return (
    <Show when={exportModal()}>
      {(m) => <Modal ids={m().ids} />}
    </Show>
  );
}

function Modal(props: { ids: string[] }): JSX.Element {
  const [opts, setOpts] = createSignal<ExportOptions>(loadOptions());
  const update = (patch: Partial<ExportOptions>) => {
    const next = { ...opts(), ...patch };
    setOpts(next);
    saveOptions(next);
  };

  // Build the node forest once (the selection is fixed while the modal is open);
  // the preview recomputes from it as options change. Rendered mode applies the
  // typographic glyphs exactly when the app displays them (not persisted).
  const nodes = exportNodesFor(props.ids);
  const text = createMemo(() =>
    exportOutline(nodes, { ...opts(), typographicGlyphs: typographyMode() === "render" })
  );

  const copy = () => {
    void backend().writeText(text());
    pushToast("Copied to clipboard", "success");
    closeExportModal();
  };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closeExportModal();
      } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        copy();
      }
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  const blockCount = props.ids.length;
  return (
    <div class="modal-overlay" onClick={closeExportModal}>
      <div class="export-modal" onClick={(e) => e.stopPropagation()}>
        <div class="export-head">
          Copy / export <span class="export-count">{blockCount} block{blockCount === 1 ? "" : "s"}</span>
        </div>

        <textarea class="export-preview" readonly spellcheck={false} value={text()} />

        <div class="export-opts">
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">Content</span>
            <For each={CONTENT_STYLES}>
              {(s) => (
                <button
                  class="export-indent-btn"
                  classList={{ active: opts().content === s.value }}
                  title={s.hint}
                  onClick={() => update({ content: s.value })}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">Indent</span>
            <For each={INDENT_STYLES}>
              {(s) => (
                <button
                  class="export-indent-btn"
                  classList={{ active: opts().indent === s.value }}
                  title={s.hint}
                  onClick={() => update({ indent: s.value })}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
          <div class="export-opt-row export-toggles">
            <For each={TOGGLES}>
              {(t) => (
                <label
                  class="export-toggle"
                  classList={{ "export-toggle-moot": t.sourceOnly && opts().content === "rendered" }}
                  title={t.sourceOnly && opts().content === "rendered" ? "Rendered text has no markup markers" : undefined}
                >
                  <input
                    type="checkbox"
                    disabled={t.sourceOnly && opts().content === "rendered"}
                    checked={opts()[t.key] as boolean}
                    onChange={(e) => update({ [t.key]: e.currentTarget.checked } as Partial<ExportOptions>)}
                  />
                  <span>{t.label}</span>
                </label>
              )}
            </For>
          </div>
        </div>

        <div class="export-foot">
          <button class="export-btn-secondary" onClick={closeExportModal}>Close</button>
          <button class="export-btn-primary" onClick={copy}>Copy</button>
        </div>
      </div>
    </div>
  );
}
