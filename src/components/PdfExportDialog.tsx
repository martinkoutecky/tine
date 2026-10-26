import { Rich } from "../i18n/Rich";
import { msg, render, t } from "../i18n";
import { optionsUpdater } from "./primitives";
import { For, Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { pdfExportPage, closePdfExport } from "../ui";
import { exportPagePdf, DEFAULT_PRINT_OPTS } from "../print";
import type { PrintOpts } from "../types";
import { registerTransientLayer } from "../transientLayers";
import { activeWindow, listen } from "../windowRealm";

// Deferred catalog messages; property getters resolve in the consuming computation.
const UI_MESSAGES = {
  "pdf-export-expand-all": msg("pdf-export-expand-all"),
  "pdf-export-print-every-block-even-folded-ones": msg("pdf-export-print-every-block-even-folded-ones"),
  "pdf-export-keep-folded": msg("pdf-export-keep-folded"),
  "pdf-export-hide-collapsed-blocks-children-as-on-screen": msg("pdf-export-hide-collapsed-blocks-children-as-on-screen"),
  "pdf-export-small": msg("pdf-export-small"),
  "pdf-export-13px": msg("pdf-export-13px"),
  "pdf-export-normal": msg("pdf-export-normal"),
  "pdf-export-16px": msg("pdf-export-16px"),
  "pdf-export-large": msg("pdf-export-large"),
  "pdf-export-19px": msg("pdf-export-19px"),
  "pdf-export-narrow": msg("pdf-export-narrow"),
  "pdf-export-10mm": msg("pdf-export-10mm"),
  "pdf-export-16mm": msg("pdf-export-16mm"),
  "pdf-export-wide": msg("pdf-export-wide"),
  "pdf-export-24mm": msg("pdf-export-24mm"),
};

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
  { value: true, get label() { return render(UI_MESSAGES["pdf-export-expand-all"]); }, get hint() { return render(UI_MESSAGES["pdf-export-print-every-block-even-folded-ones"]); } },
  { value: false, get label() { return render(UI_MESSAGES["pdf-export-keep-folded"]); }, get hint() { return render(UI_MESSAGES["pdf-export-hide-collapsed-blocks-children-as-on-screen"]); } },
];
const FONT: { value: number; label: string; hint: string }[] = [
  { value: 13, get label() { return render(UI_MESSAGES["pdf-export-small"]); }, get hint() { return render(UI_MESSAGES["pdf-export-13px"]); } },
  { value: 16, get label() { return render(UI_MESSAGES["pdf-export-normal"]); }, get hint() { return render(UI_MESSAGES["pdf-export-16px"]); } },
  { value: 19, get label() { return render(UI_MESSAGES["pdf-export-large"]); }, get hint() { return render(UI_MESSAGES["pdf-export-19px"]); } },
];
const MARGIN: { value: number; label: string; hint: string }[] = [
  { value: 10, get label() { return render(UI_MESSAGES["pdf-export-narrow"]); }, get hint() { return render(UI_MESSAGES["pdf-export-10mm"]); } },
  { value: 16, get label() { return render(UI_MESSAGES["pdf-export-normal"]); }, get hint() { return render(UI_MESSAGES["pdf-export-16mm"]); } },
  { value: 24, get label() { return render(UI_MESSAGES["pdf-export-wide"]); }, get hint() { return render(UI_MESSAGES["pdf-export-24mm"]); } },
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
  const update = optionsUpdater(opts, setOpts, saveOpts);

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
    onCleanup(listen(activeWindow(), "keydown", onKey, true));
  });

  return (
    <div class="modal-overlay" onClick={closePdfExport}>
      <div ref={root} class="export-modal pdf-export-modal" onClick={(e) => e.stopPropagation()}>
        <div class="export-head">
          {t("pdf-export-export-to-pdf")}<span class="export-count">{props.name}</span>
        </div>

        <div class="export-opts">
          <div class="export-opt-row export-indent">
            <span class="export-opt-label">{t("pdf-export-collapsed-blocks")}</span>
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
            <span class="export-opt-label">{t("pdf-export-font-size")}</span>
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
            <span class="export-opt-label">{t("pdf-export-margins")}</span>
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
          <Rich message={msg("pdf-export-print-note")} slots={{ strong: children => <b>{children}</b> }} />
        </div>

        <div class="export-foot">
          <button class="export-btn-secondary" onClick={closePdfExport}>{t("pdf-export-cancel")}</button>
          <button class="export-btn-primary" onClick={doExport}>{t("pdf-export-export")}</button>
        </div>
      </div>
    </div>
  );
}
