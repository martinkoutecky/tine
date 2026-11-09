import { t } from "../i18n";
import { settings } from "./settingDefinitions";
import { Field, Toggle } from "./settingsField";
import { commentMarginHidden } from "../commentMarginPref";
import { toggleCommentMargin } from "../commentMargin";
import { authorMarksHidden, toggleAuthorMarks } from "../authorMarksPref";

export function AnnotationVisibilitySettings() {
  return <div role="group" aria-label={t("settings-annotation-visibility")}>
    <Field setting={settings["settings-ui-comment-margin"]}>
      <Toggle on={!commentMarginHidden()} onClick={toggleCommentMargin} />
    </Field>
    <Field setting={settings["settings-author-marks"]}>
      <Toggle on={!authorMarksHidden()} onClick={toggleAuthorMarks} />
    </Field>
  </div>;
}
