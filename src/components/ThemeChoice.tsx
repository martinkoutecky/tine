import { t } from "../i18n";
import { createMemo, type JSX } from "solid-js";
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

/** Escape external values before inserting them in a rich catalog message.
 * The renderer decodes them only after selecting the catalog's inline slots. */
export function richValue(value: string | number): string | number {
  return typeof value === "number" ? value : value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

/** Catalog markup maps only to caller-owned inline elements. Unknown markup
 * stays text; neither catalog text nor message variables can create HTML. */
export function Rich(props: { text: string; slots: Record<string, (children: JSX.Element) => JSX.Element> }): JSX.Element {
  const decode = (text: string) => text.replaceAll("&lt;", "<").replaceAll("&gt;", ">").replaceAll("&amp;", "&");
  const parse = (text: string): JSX.Element => {
    const result: JSX.Element[] = [];
    const tags = /<([a-z][a-z0-9]*)>([\s\S]*?)<\/\1>/g;
    let start = 0;
    for (const match of text.matchAll(tags)) {
      result.push(decode(text.slice(start, match.index)));
      const slot = props.slots[match[1]];
      result.push(slot ? slot(parse(match[2])) : decode(match[0]));
      start = match.index + match[0].length;
    }
    result.push(decode(text.slice(start)));
    return result;
  };
  const content = createMemo(() => parse(props.text));
  return <>{content()}</>;
}
