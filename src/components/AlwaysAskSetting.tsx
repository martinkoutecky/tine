import type { JSX } from "solid-js";
import { conflictPolicyAlwaysAsk, setConflictPolicyAlwaysAsk } from "../conflictPolicy";
import { Field, Toggle } from "./settingsField";

/** Settings → Backups & recovery: hold even clean external changes for review
 *  (src/conflictPolicy.ts; Guide "Files, external edits, and backups"). */
export function AlwaysAskSetting(): JSX.Element {
  return (
    <Field
      label="Always ask before applying an external change"
      hint="When a page you are not editing changes on disk, keep what you are reading and offer Reload from disk / Keep mine instead of updating it. Changes that conflict or arrive mid-edit are always handled safely either way."
    >
      <Toggle on={conflictPolicyAlwaysAsk()} onClick={() => setConflictPolicyAlwaysAsk(!conflictPolicyAlwaysAsk())} />
    </Field>
  );
}
