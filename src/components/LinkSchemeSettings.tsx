import { For, Show, onMount } from "solid-js";
import { loadLinkSchemeSettings, rememberedLinkSchemes, setLinkSchemeRemembered } from "../linkSchemeSettings";
import { t } from "../i18n";
import { Field } from "./settingsField";

export function LinkSchemeSettings() {
  onMount(() => void loadLinkSchemeSettings());
  return <Field label={t("links-remembered-schemes")} hint={t("links-consent-device-hint")}>
    <div class="link-scheme-settings">
      <For each={rememberedLinkSchemes()}>{scheme => <div>
        <code>{scheme}:</code>{" "}
        <button type="button" class="settings-btn" aria-label={t("links-forget-scheme", { scheme })}
          onClick={() => setLinkSchemeRemembered(scheme, false)}>{t("links-forget")}</button>
      </div>}</For>
      <Show when={rememberedLinkSchemes().length === 0}><span>{t("links-no-remembered-schemes")}</span></Show>
    </div>
  </Field>;
}
