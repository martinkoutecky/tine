import { Show, createEffect, onCleanup, type JSX } from "solid-js";
import { cssEditorSession, cssEditorText, setCssEditorText, cssEditorProblem, cssEditorBusy, flushCssEditor, closeCssEditor, closeCssEditorOnOutsideClick, cancelCssEditor, reloadCssEditor, checkCssEditorDisk } from "../cssEditor";
import { DialogActions } from "./DialogActions";
import { registerTransientLayer } from "../transientLayers";
import { t } from "../i18n";
import { graphTransitioning } from "../ui";
import { onEachWindow } from "../windowRealm";

export function CssEditor(): JSX.Element {
  createEffect(() => {
    if (!cssEditorSession()) return;
    const unregister = registerTransientLayer({ id: "css-editor", dismiss: () => { void cancelCssEditor(); return true; } });
    onCleanup(unregister);
    const focus = () => { void checkCssEditorDisk(cssEditorSession()?.global ?? false); };
    onCleanup(onEachWindow((win) => {
      win.addEventListener("focus", focus);
      return () => win.removeEventListener("focus", focus);
    }));
  });
  return <Show when={cssEditorSession()}>{(session) =>
    <div class="modal-overlay" onClick={() => void closeCssEditorOnOutsideClick()}>
      <div class="settings-modal" role="dialog" aria-modal="true" aria-label={t("css-editor-title")}
        style={{ "flex-direction": "column", padding: "20px", gap: "12px" }} onClick={(event) => event.stopPropagation()}>
        <h2 style={{ margin: "0" }}>{session().global ? t("css-global-file") : t("css-graph-file")}</h2>
        <textarea aria-label={t("css-editor-title")} spellcheck={false} value={cssEditorText()} disabled={cssEditorBusy() || graphTransitioning()}
          onInput={(event) => setCssEditorText(event.currentTarget.value)}
          style={{ width: "100%", "min-height": "25vh", "font-family": "monospace", flex: "1", "box-sizing": "border-box" }} />
        <Show when={cssEditorProblem()}><div role="alert" style={{ "max-height": "20vh", overflow: "auto" }}>{cssEditorProblem()}</div></Show>
        <DialogActions
          aux={<>
            <button class="settings-btn" title={t("css-reload-title")} disabled={cssEditorBusy()} onClick={() => void reloadCssEditor()}>{t("css-reload")}</button>
            <button class="settings-btn" disabled={cssEditorBusy()} onClick={() => void flushCssEditor()}>{t("css-save")}</button>
          </>}
          cancel={<button class="settings-btn" disabled={cssEditorBusy()} onClick={() => void cancelCssEditor()}>{t("css-cancel")}</button>}
          primary={<button class="settings-btn settings-btn-primary" disabled={cssEditorBusy()} onClick={() => void closeCssEditor()}>{t("css-save-close")}</button>} />
      </div>
    </div>
  }</Show>;
}
