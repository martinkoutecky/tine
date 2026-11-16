import { t } from "../i18n";

/**
 * The one disk-vs-mine choice (D-20261018-UI2a): [Keep mine][Use disk version],
 * these labels and this order everywhere (conflict bar, external-change bar,
 * PDF highlight conflict). Neither button is styled primary or danger: both
 * lose something, so neither is the default. `diskOrMineChoice.guard.test.ts`
 * pins that the labels have no other owner.
 */
export function DiskOrMineChoice(props: {
  /** Button class of the host surface (`conflict-btn`, `settings-btn`). */
  buttonClass: string;
  disabled?: boolean;
  onKeepMine: () => void;
  onUseDisk: () => void;
}) {
  return (
    <>
      <button class={props.buttonClass} data-choice="keep-mine" disabled={props.disabled} onClick={props.onKeepMine}>{t("conflict-choice-keep-mine")}</button>
      <button class={props.buttonClass} data-choice="use-disk" disabled={props.disabled} onClick={props.onUseDisk}>{t("conflict-choice-use-disk-version")}</button>
    </>
  );
}
