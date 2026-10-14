import { Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { customCssDisabled, setCustomCssDisabled } from "../customCss";
import { ownedWhen, readOwned } from "../owned";
import { openCssEditor } from "../cssEditor";
import { msg, t } from "../i18n";
import { platformKind } from "../platform";
import { isMac } from "../nativeChrome";
import { openDevtools } from "../ui";
import { Field, Toggle } from "./settingsField";
import { Rich } from "../i18n/Rich";

/** Settings > Appearance > Custom CSS (GH #610/#372): the affordances that
 *  make graph and device CSS discoverable and recoverable.
 *  - Both fixed files open in Tine's guarded text editor on every platform.
 *  - Live reload needs no control: the watcher re-applies outside edits.
 *  - Disable custom CSS: a session-only safe mode (never persisted).
 *  - Developer tools (desktop): the inspector, to find what styles an element. */
export function CustomCssSettings(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  const [desktop, setDesktop] = createSignal(false);
  onMount(() => {
    void readOwned(ownedWhen(() => alive), platformKind()).then((result) => {
      if (result.kind === "current") setDesktop(result.value === "desktop");
    });
  });
  return (
    <><div class="settings-section">{t("settings-custom-css")}</div>
        <Field
          label={t("css-edit-graph")}
          hint={t("css-graph-hint")}
        >
          <button class="settings-btn" onClick={() => void openCssEditor(false)}>
            {t("css-edit-graph")}
          </button>
        </Field>
      <Field label={t("css-edit-global")} hint={t("css-global-hint")}>
        <button class="settings-btn" onClick={() => void openCssEditor(true)}>{t("css-edit-global")}</button>
      </Field>
      <Field
        label={t("settings-search-disable-custom-css")}
        hint={t("css-safe-hint")}
      >
        <Toggle on={customCssDisabled()} onClick={() => setCustomCssDisabled(!customCssDisabled())} />
      </Field>
      <Show when={desktop()}>
        <Field
          label={t("settings-search-developer-tools")}
          hint={<Rich message={msg("settings-inspector-hint", { platform: isMac ? "mac" : "other" })} slots={{ "kbd": (children) => <kbd>{children}</kbd> }} />}
        ><button class="settings-btn" onClick={() => openDevtools()}>{t("settings-open-developer-tools")}</button>
        </Field>
      </Show>
    </>
  );
}
