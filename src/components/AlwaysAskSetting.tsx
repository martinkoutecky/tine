import { t } from "../i18n";
import type { JSX } from "solid-js";
import { conflictPolicyAlwaysAsk, setConflictPolicyAlwaysAsk } from "../conflictPolicy";
import { Field, Toggle } from "./settingsField";

/** Settings → Backups & recovery: hold even clean external changes for review
 *  (src/conflictPolicy.ts; Guide "Files, external edits, and backups"). */
export function AlwaysAskSetting(): JSX.Element {
  return (
    <Field
      label={t("settings-prose-always-ask-before-applying-an-external-change")}
      hint={t("settings-prose-when-a-page-you-are-not-editing-changes-on-disk")}
    >
      <Toggle on={conflictPolicyAlwaysAsk()} onClick={() => setConflictPolicyAlwaysAsk(!conflictPolicyAlwaysAsk())} />
    </Field>
  );
}
