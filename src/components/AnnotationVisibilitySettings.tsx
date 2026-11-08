import { t } from "../i18n";
import { settings } from "./settingDefinitions";
import { Field, Toggle } from "./settingsField";
import { commentMarginHidden, commentsOnClick, changeCommentsOnClick } from "../commentMarginPref";
import { toggleCommentMargin } from "../commentMargin";
import { authorMarksHidden, toggleAuthorMarks } from "../authorMarksPref";

export function AnnotationVisibilitySettings() {
  return <div role="group" aria-label={t("settings-annotation-visibility")}>
    <Field setting={settings["settings-comments-display"]}>
      <select class="settings-select" aria-label={t("settings-comments-display")} value={commentsOnClick() ? "click" : "margin"}
        onChange={event => {
          changeCommentsOnClick(event.currentTarget.value === "click");
          event.currentTarget.value = commentsOnClick() ? "click" : "margin";
        }}>
        <option value="margin">{t("settings-comments-in-margin")}</option>
        <option value="click">{t("settings-comments-on-click")}</option>
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
