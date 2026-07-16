import { For, Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { pdfExportPage, closePdfExport } from "../ui";
import { exportPagePdf, DEFAULT_PRINT_OPTS } from "../print";
import type { PrintOpts } from "../types";
import { registerTransientLayer } from "../transientLayers";

const STORE_KEY = "tine.pdfExportOpts";

// Persist the last-used options so the dialog opens the way you left it.
function loadOpts(): PrintOpts {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw) return { ...DEFAULT_PRINT_OPTS, ...JSON.parse(raw) };
  } catch {
    /* ignore malformed/missing */
  }
  return { ...DEFAULT_PRINT_OPTS };
}
function saveOpts(o: PrintOpts): void {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(o));
  } catch {
    /* storage unavailable — non-fatal */
  }
}

const COLLAPSED: { value: boolean; label: string; hint: string }[] = [
  { value: true, label: "Expand all", hint: "print every block, even folded ones" },
  { value: false, label: "Keep folded", hint: "hide collapsed blocks' children, as on screen" },
];
const FONT: { value: number; label: string; hint: string }[] = [
  { value: 13, label: "Small", hint: "13px" },
  { value: 16, label: "Normal", hint: "16px" },
  { value: 19, label: "Large", hint: "19px" },
];
const MARGIN: { value: number; label: string; hint: string }[] = [
  { value: 10, label: "Narrow", hint: "10mm" },
  { value: 16, label: "Normal", hint: "16mm" },
  { value: 24, label: "Wide", hint: "24mm" },
];

/** Pre-export options dialog for "Export to PDF…". Collects render options, then
 *  hands off to the OS print dialog (→ Save as PDF). */
export function PdfExportDialog(): JSX.Element {
  return (
    <Show when={pdfExportPage()}>
      {(name) => <Dialog name={name()} />}
    </Show>
  );
}

function Dialog(props: { name: string }): JSX.Element {
  let root: HTMLDivElement | undefined;
  createEffect(() => {
    const unregister = registerTransientLayer({ id: "pdf-export", root: () => root ?? null, dismiss: () => { closePdfExport(); return true; } });
    onCleanup(unregister);
  });
  const [opts, setOpts] = createSignal<PrintOpts>(loadOpts());
  const update = (patch: Partial<PrintOpts>) => {
    const next = { ...opts(), ...patch };
    setOpts(next);
    saveOpts(next);
  };

  const doExport = () => {
    const name = props.name;
    const o = opts();
    closePdfExport();
    void exportPagePdf(name, o);
  };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        doExport();
      }
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  return (
    <div class="modal-overlay" onClick={closePdfExport}>
      <div ref={root} class="export-modal pdf-export-modal" onClick={(e) => e.stopPropagation()}>
        <div class="export-head">
          Export to PDF <span class="export-count">{props.name}</span>
        </div>

        <div class="export-opts">
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">Collapsed blocks</span>
            <For each={COLLAPSED}>
              {(s) => (
                <button
                  class="export-indent-btn"
                  classList={{ active: opts().expand_collapsed === s.value }}
                  title={s.hint}
                  onClick={() => update({ expand_collapsed: s.value })}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">Font size</span>
            <For each={FONT}>
              {(s) => (
                <button
                  class="export-indent-btn"
                  classList={{ active: opts().font_px === s.value }}
                  title={s.hint}
                  onClick={() => update({ font_px: s.value })}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">Margins</span>
            <For each={MARGIN}>
              {(s) => (
                <button
                  class="export-indent-btn"
                  classList={{ active: opts().margin_mm === s.value }}
                  title={s.hint}
                  onClick={() => update({ margin_mm: s.value })}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
        </div>

        <div class="pdf-export-note">
          Opens your system print dialog — choose <b>Save as PDF</b> (or a printer).
          The document always prints on a light background, whatever your theme.
        </div>

        <div class="export-foot">
          <button class="export-btn-secondary" onClick={closePdfExport}>Cancel</button>
          <button class="export-btn-primary" onClick={doExport}>Export…</button>
        </div>
      </div>
    </div>
  );
}
