import { t } from "../i18n";
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
          label={t("content-width-fields-standard-page-width")}
          hint={t("content-width-fields-maximum-reading-column-width-on-this-device-reset")}
        >
          <div class="settings-width-control">
            <input
              class="settings-width-range"
              aria-label={t("content-width-fields-standard-page-width")}
              type="range"
              min={MIN_CONTENT_WIDTH}
              max={CONTENT_WIDTH_SLIDER_MAX}
              step="10"
              value={standardContentWidth() ?? DEFAULT_STANDARD_CONTENT_WIDTH}
              onInput={(event) => changeStandardContentWidth(event.currentTarget.valueAsNumber)}
            />
            <input
              class="settings-num settings-width-number"
              aria-label={t("content-width-fields-standard-page-width-in-pixels")}
              type="number"
              min={MIN_CONTENT_WIDTH}
              max={MAX_CONTENT_WIDTH}
              step="10"
              value={standardContentWidth() ?? DEFAULT_STANDARD_CONTENT_WIDTH}
              onChange={(event) => changeStandardContentWidth(event.currentTarget.valueAsNumber)}
            />
            <span class="settings-width-unit">{t("content-width-fields-px")}</span>
            <Show when={standardContentWidth() !== null}>
              <button class="settings-btn" onClick={resetStandardContentWidth}>{t("content-width-fields-reset")}</button>
            </Show>
          </div>
        </Field>

        <Field
          label={t("content-width-fields-wide-page-width")}
          hint={t("content-width-fields-wide-mode-can-fill-the-available-pane-or")}
        >
          <div class="settings-width-control">
            <select
              class="settings-select"
              aria-label={t("content-width-fields-wide-page-width-mode")}
              value={wideContentWidth() === null ? "fill" : "custom"}
              onChange={(event) =>
                changeWideContentWidth(
                  event.currentTarget.value === "fill"
                    ? null
                    : (wideContentWidth() ?? DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH),
                )
              }
            >
              <option value="fill">{t("content-width-fields-fill-pane")}</option>
              <option value="custom">{t("content-width-fields-custom-maximum")}</option>
            </select>
            <Show when={wideContentWidth() !== null}>
              <input
                class="settings-width-range"
                aria-label={t("content-width-fields-wide-page-width")}
                type="range"
                min={MIN_CONTENT_WIDTH}
                max={CONTENT_WIDTH_SLIDER_MAX}
                step="10"
                value={wideContentWidth() ?? DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH}
                onInput={(event) => changeWideContentWidth(event.currentTarget.valueAsNumber)}
              />
              <input
                class="settings-num settings-width-number"
                aria-label={t("content-width-fields-wide-page-width-in-pixels")}
                type="number"
                min={MIN_CONTENT_WIDTH}
                max={MAX_CONTENT_WIDTH}
                step="10"
                value={wideContentWidth() ?? DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH}
                onChange={(event) => changeWideContentWidth(event.currentTarget.valueAsNumber)}
              />
              <span class="settings-width-unit">{t("content-width-fields-px")}</span>
            </Show>
          </div>
        </Field>
    </>
  );
}
