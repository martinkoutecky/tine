import { Show, type JSX } from "solid-js";

// One setting: label + control on a line, with the explanatory hint on its own
// full-width line below (so long hints read cleanly instead of being squeezed
// into the right column). Pass `hint` as JSX to allow inline <code>/markup.
export function Field(props: { label: string; hint?: JSX.Element; children: JSX.Element }): JSX.Element {
  return (
    <div class="settings-field" data-setting-label={props.label}>
      <div class="settings-field-row">
        <span class="settings-label">{props.label}</span>
        <div class="settings-field-control">{props.children}</div>
      </div>
      <Show when={props.hint}>
        <div class="settings-hint settings-field-hint">{props.hint}</div>
      </Show>
    </div>
  );
}
