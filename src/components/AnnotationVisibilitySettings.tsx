import { t } from "../i18n";
import { settings } from "./settingDefinitions";
import { Field, Toggle } from "./settingsField";
import { commentMarginHidden, commentsInPopup, changeCommentsInPopup } from "../commentMarginPref";
import { toggleCommentMargin } from "../commentMargin";
import { authorMarksHidden, toggleAuthorMarks } from "../authorMarksPref";

export function AnnotationVisibilitySettings() {
  return <div role="group" aria-label={t("settings-annotation-visibility")}>
    <Field setting={settings["settings-comments-display"]}>
      <select class="settings-select" aria-label={t("settings-comments-display")} value={commentsInPopup() ? "popup" : "margin"}
        onChange={event => {
          changeCommentsInPopup(event.currentTarget.value === "popup");
          event.currentTarget.value = commentsInPopup() ? "popup" : "margin";
        }}>
        <option value="margin">{t("settings-comments-in-margin")}</option>
        <option value="popup">{t("settings-comments-in-popup")}</option>
      </select>
    </Field>
    <Field setting={settings["settings-ui-comment-margin"]}>
      <Toggle on={!commentMarginHidden()} onClick={toggleCommentMargin} />
    </Field>
    <Field setting={settings["settings-author-marks"]}>
      <Toggle on={!authorMarksHidden()} onClick={toggleAuthorMarks} />
    </Field>
  </div>;
}
