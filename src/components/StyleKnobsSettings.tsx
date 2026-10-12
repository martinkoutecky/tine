import { For, Show, type JSX } from "solid-js";
import {
  KNOBS,
  knobIsSet,
  setKnob,
  styleKnobValues,
  validKnobValue,
  type StyleKnob,
} from "../styleKnobs";
import { Field, Toggle } from "./settingsField";

/** Settings > Appearance > Fine-tuning (GH #610 item 3): one control per row of
 *  the style-knob table, rendered by type. No code per knob (K1): a new visual
 *  detail is a row in `KNOBS` (src/styleKnobs.ts). Each knob has a Reset that
 *  returns to Tine's default look. Saved on this device. */
type Control = Exclude<StyleKnob, { type: "heading" }>;

function KnobControl(props: { knob: Control }): JSX.Element {
  const knob = props.knob;
  const stored = () => validKnobValue(knob, styleKnobValues()[knob.id]);
  switch (knob.type) {
    case "class-toggle":
      return <Toggle on={stored() === true} onClick={() => setKnob(knob, stored() !== true)} />;
    case "class-select":
    case "variable-select":
      return (
        <select
          class="settings-select"
          aria-label={knob.title}
          value={(stored() as string | null) ?? knob.default}
          onChange={(event) => setKnob(knob, event.currentTarget.value)}
        >
          <For each={knob.options}>{(option) => <option value={option.value}>{option.label}</option>}</For>
        </select>
      );
    case "variable-number-slider":
      return (
        <span class="knob-slider">
          <input
            type="range"
            aria-label={knob.title}
            min={knob.min}
            max={knob.max}
            step={knob.step}
            value={(stored() as number | null) ?? knob.default}
            onInput={(event) => setKnob(knob, Number(event.currentTarget.value))}
          />
          <span class="mono knob-slider-value">{(stored() as number | null) ?? knob.default}{knob.format ?? ""}</span>
        </span>
      );
    case "variable-color": {
      const current = () => (stored() as string | null) ?? knob.default;
      const alpha = () => (current().length === 9 ? parseInt(current().slice(7), 16) : 255);
      const rgbOf = (hex: string) => (hex.length === 3 ? hex.replace(/./g, "$&$&") : hex);
      const rgb = () => rgbOf(current().slice(1, 7));
      const combine = (color: string, a: number) =>
        knob.opacity && a < 255 ? `#${rgbOf(color)}${a.toString(16).padStart(2, "0")}` : `#${rgbOf(color)}`;
      return (
        <span class="knob-color">
          <input
            type="color"
            class="settings-color"
            aria-label={knob.title}
            value={`#${rgb()}`}
            onInput={(event) => setKnob(knob, combine(event.currentTarget.value.slice(1), alpha()))}
          />
          <Show when={knob.opacity}>
            <input
              type="range"
              aria-label={`${knob.title} opacity`}
              min={0}
              max={255}
              step={1}
              value={alpha()}
              onInput={(event) => setKnob(knob, combine(rgb(), Number(event.currentTarget.value)))}
            />
          </Show>
        </span>
      );
    }
  }
}

export function StyleKnobsSettings(props: { knobs?: readonly StyleKnob[] }): JSX.Element {
  const table = () => props.knobs ?? KNOBS;
  return (
    <>
      <div class="settings-section">Fine-tuning</div>
      <div class="settings-hint theme-gallery-hint">
        Small visual details, saved on this device. Each starts at Tine's default look; your <code>logseq/custom.css</code> still takes priority, and Disable custom CSS below switches these off for the session too.
      </div>
      <For each={table()}>
        {(knob) =>
          knob.type === "heading" ? (
            <div class="settings-subsection" data-knob-heading={knob.id}>{knob.title}</div>
          ) : (
            <Field label={knob.title} hint={knob.description}>
              <KnobControl knob={knob} />
              <Show when={knobIsSet(knob, styleKnobValues())}>
                <button class="settings-btn knob-reset" aria-label={`Reset ${knob.title}`} onClick={() => setKnob(knob, undefined)}>
                  Reset
                </button>
              </Show>
            </Field>
          )
        }
      </For>
    </>
  );
}
