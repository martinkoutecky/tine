import { For, type JSX } from "solid-js";
import { LOCALES, negotiateLocale, t } from "../i18n";
import { setUiLanguage, systemLanguages, uiLanguagePreference } from "../i18n/preference";

/** Settings → Appearance: the UI language (tine#107). Each language is listed by
 * its own name; "System default" names the language the system resolves to. A
 * device preference; the graph's content is never translated. */
export function LanguageSetting(): JSX.Element {
  const systemName = () => LOCALES.find((entry) => entry.tag === negotiateLocale("", systemLanguages()))?.name ?? "English";
  return (
    // The search join key stays English (settingsSearch.ts); the label is translated.
    <div class="settings-field" data-setting-label="Language">
      <div class="settings-field-row">
        <span class="settings-label">{t("settings-language-label")}</span>
        <div class="settings-field-control">
          <select
            class="settings-select"
            aria-label={t("settings-language-label")}
            value={uiLanguagePreference()}
            onChange={(event) => setUiLanguage(event.currentTarget.value)}
          >
            <option value="">{t("settings-language-system", { language: systemName() })}</option>
            <For each={LOCALES}>{(entry) => <option value={entry.tag} lang={entry.tag}>{entry.name}</option>}</For>
          </select>
        </div>
      </div>
      <div class="settings-hint settings-field-hint">{t("settings-language-description")}</div>
    </div>
  );
}
