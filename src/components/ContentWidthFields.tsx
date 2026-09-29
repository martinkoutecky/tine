import { Show, type JSX } from "solid-js";
import {
  CONTENT_WIDTH_SLIDER_MAX,
  DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH,
  DEFAULT_STANDARD_CONTENT_WIDTH,
  MAX_CONTENT_WIDTH,
  MIN_CONTENT_WIDTH,
  changeStandardContentWidth,
  changeWideContentWidth,
  resetStandardContentWidth,
  standardContentWidth,
  wideContentWidth,
} from "../contentWidth";
import { Field } from "./settingsField";

// Settings → Appearance → Advanced: the device-local reading-column widths
// (GH #382). Themes keep ownership of the defaults; only an explicit override
// is stored.
export function ContentWidthFields(): JSX.Element {
  return (
    <>
        <Field
          label="Standard page width"
          hint="Maximum reading-column width on this device. Reset uses the active theme's default (810 px in Tine's built-in themes)."
        >
          <div class="settings-width-control">
            <input
              class="settings-width-range"
              aria-label="Standard page width"
              type="range"
              min={MIN_CONTENT_WIDTH}
              max={CONTENT_WIDTH_SLIDER_MAX}
              step="10"
              value={standardContentWidth() ?? DEFAULT_STANDARD_CONTENT_WIDTH}
              onInput={(event) => changeStandardContentWidth(event.currentTarget.valueAsNumber)}
            />
            <input
              class="settings-num settings-width-number"
              aria-label="Standard page width in pixels"
              type="number"
              min={MIN_CONTENT_WIDTH}
              max={MAX_CONTENT_WIDTH}
              step="10"
              value={standardContentWidth() ?? DEFAULT_STANDARD_CONTENT_WIDTH}
              onChange={(event) => changeStandardContentWidth(event.currentTarget.valueAsNumber)}
            />
            <span class="settings-width-unit">px</span>
            <Show when={standardContentWidth() !== null}>
              <button class="settings-btn" onClick={resetStandardContentWidth}>Reset</button>
            </Show>
          </div>
        </Field>

        <Field
          label="Wide page width"
          hint="Wide mode can fill the available pane or stop at a custom maximum. Saved on this device."
        >
          <div class="settings-width-control">
            <select
              class="settings-select"
              aria-label="Wide page width mode"
              value={wideContentWidth() === null ? "fill" : "custom"}
              onChange={(event) =>
                changeWideContentWidth(
                  event.currentTarget.value === "fill"
                    ? null
                    : (wideContentWidth() ?? DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH),
                )
              }
            >
              <option value="fill">Fill pane</option>
              <option value="custom">Custom maximum</option>
            </select>
            <Show when={wideContentWidth() !== null}>
              <input
                class="settings-width-range"
                aria-label="Wide page width"
                type="range"
                min={MIN_CONTENT_WIDTH}
                max={CONTENT_WIDTH_SLIDER_MAX}
                step="10"
                value={wideContentWidth() ?? DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH}
                onInput={(event) => changeWideContentWidth(event.currentTarget.valueAsNumber)}
              />
              <input
                class="settings-num settings-width-number"
                aria-label="Wide page width in pixels"
                type="number"
                min={MIN_CONTENT_WIDTH}
                max={MAX_CONTENT_WIDTH}
                step="10"
                value={wideContentWidth() ?? DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH}
                onChange={(event) => changeWideContentWidth(event.currentTarget.valueAsNumber)}
              />
              <span class="settings-width-unit">px</span>
            </Show>
          </div>
        </Field>
    </>
  );
}
