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
      role="radiogroup" aria-label="Appearance">
      {option("light", "☀", "Light")}
      {option("system", "◐", "System")}
      {option("dark", "☾", "Dark")}
      <span class="theme-knob" />
    </div>
  );
}
