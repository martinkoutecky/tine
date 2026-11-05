import { Show, type JSX } from "solid-js";
import { render, type Msg } from "../i18n";
import { Rich, richPlain } from "../i18n/Rich";
import type { SettingDescriptor } from "./settingsDescriptor";

export const settingLabel = (setting: SettingDescriptor): string =>
  typeof setting.label === "string" ? setting.label : richPlain(render(setting.label));
export function settingHint(setting: SettingDescriptor, args?: Msg["args"]): Msg {
  if (!setting.description || typeof setting.description === "string") throw new Error(`Setting ${setting.id} has no catalog description`);
  return { ...setting.description, args: args ?? setting.description.args };
}
export const settingDescription = (setting: SettingDescriptor, args?: Msg["args"]): string =>
  typeof setting.description === "string" ? setting.description : setting.description ? richPlain(render(settingHint(setting, args))) : "";
/** data-setting-label remains a compatibility hook for tools; identity and
 * navigation depend exclusively on data-setting-id. */
export const settingAttributes = (setting: SettingDescriptor) => ({
  "data-setting-id": setting.id, "data-setting-label": settingLabel(setting),
});

// One setting: label + control on a line, with the explanatory hint on its own
// full-width line below (so long hints read cleanly instead of being squeezed
// into the right column). Pass `hint` as JSX to allow inline <code>/markup.
export function Field(props: { setting: SettingDescriptor; hint?: JSX.Element; children: JSX.Element }): JSX.Element {
  const hint = () => props.hint ?? (props.setting.description
    ? typeof props.setting.description === "string" ? props.setting.description : <Rich message={props.setting.description} />
    : undefined);
  return (
    <div class="settings-field" {...settingAttributes(props.setting)}>
      <div class="settings-field-row">
        <span class="settings-label">{settingLabel(props.setting)}</span>
        <div class="settings-field-control">{props.children}</div>
      </div>
      <Show when={hint()}>
        <div class="settings-hint settings-field-hint">{hint()}</div>
      </Show>
    </div>
  );
}

/** The on/off switch every boolean setting uses. */
export function Toggle(props: { on: boolean; onClick: () => void; disabled?: boolean }): JSX.Element {
  return (
    <button
      class="settings-toggle"
      classList={{ on: props.on }}
      role="switch"
      aria-checked={props.on}
      disabled={props.disabled}
      onClick={props.onClick}
    >
      <span class="settings-toggle-knob" />
    </button>
  );
}
