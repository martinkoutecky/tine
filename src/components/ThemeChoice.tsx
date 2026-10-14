import { t } from "../i18n";
import type { JSX } from "solid-js";
import { appearancePreference, setAppearancePreference, type ThemePreference } from "../ui";

/** Three stored appearance choices. The System option follows OS changes
 * through the shared resolved-theme signal; no graph data is read or written. */
export function ThemeChoice(): JSX.Element {
  const option = (value: ThemePreference, icon: string, label: string) => (
    <button type="button" class="theme-opt" role="radio"
      aria-checked={appearancePreference() === value}
      onClick={() => setAppearancePreference(value)}>
      <span class="theme-ico">{icon}</span>{label}
    </button>
  );
  return (
    <div class="theme-switch theme-switch3"
      classList={{ "is-light": appearancePreference() === "light", "is-system": appearancePreference() === "system", "is-dark": appearancePreference() === "dark" }}
      role="radiogroup" aria-label={t("settings-appearance")}>
      {option("light", "☀", t("settings-theme-light"))}
      {option("system", "◐", t("settings-theme-system"))}
      {option("dark", "☾", t("settings-theme-dark"))}
      <span class="theme-knob" />
    </div>
  );
}
