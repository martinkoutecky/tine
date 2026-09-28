import { Show, createSignal, createUniqueId, onMount, type JSX } from "solid-js";

/** **The user half of the persisted-form crossing (SPEC §4.3 "Notice", §7.5).** The mechanical half already … */
export function CrossingNotice(props: {
  /** Whether the change this notice offers to take back is still the change the ordinary Undo would take back. */
  canUndo: boolean;
  /** The §4.3 seam for NAMING the unsupported feature. */
  feature?: string;
  /** **A bounded excerpt of the text the engine just printed (P4).** Not the same claim as `feature`, which is … */
  changed?: string;
  onUndo: () => void;
  onKeep: () => void;
  /** Called at most once, on close, when "Don't show this again" is ticked. */
  onDontShowAgain: () => void;
  /** **The checkbox, optionally lifted (P4, N3).** P4 moves this one notice between two hosts — inline under … */
  dontShow?: boolean;
  onDontShowChange?: (value: boolean) => void;
  /** Whether to take focus on mount. */
  autoFocus?: boolean;
  onFocused?: () => void;
}): JSX.Element {
  const [ownDontShow, setOwnDontShow] = createSignal(false);
  const dontShow = () => props.dontShow ?? ownDontShow();
  const setDontShow = (value: boolean) => {
    if (props.dontShow === undefined) setOwnDontShow(value);
    props.onDontShowChange?.(value);
  };
  const checkboxId = `crossing-notice-dont-show-${createUniqueId()}`;
  let region: HTMLDivElement | undefined;

  // The bytes already changed; a notice the user's eyes never reach is the same as no notice.
  onMount(() => {
    if (props.autoFocus === false) return;
    region?.focus();
    props.onFocused?.();
  });

  const close = (act: () => void) => {
    if (dontShow()) props.onDontShowAgain();
    act();
  };

  return (
    <div
      ref={region}
      class="query-crossing-notice"
      role="status"
      tabindex="-1"
      onClick={(e) => e.stopPropagation()}
    >
      <p class="query-crossing-notice-text">
        This query now uses Tine features Logseq can't read
        <Show when={props.feature}>{(feature) => <> (<code>{feature()}</code>)</>}</Show>. Logseq
        will show the block as plain text.
      </p>
      <Show when={props.changed}>
        {(text) => (
          <p class="query-crossing-notice-changed">
            The block now reads: <code>{text()}</code>
          </p>
        )}
      </Show>
      <div class="query-crossing-notice-actions">
        <button
          class="query-crossing-notice-undo"
          disabled={!props.canUndo}
          title={
            props.canUndo
              ? "Put the query back the way it was"
              : "Something else was changed since; use the ordinary Undo to step back to it"
          }
          onClick={() => close(props.onUndo)}
        >
          {props.canUndo ? "Undo that change" : "Undo (use Ctrl+Z)"}
        </button>
        <button class="query-crossing-notice-keep" onClick={() => close(props.onKeep)}>
          Keep it
        </button>
        <label class="query-crossing-notice-dismiss" for={checkboxId}>
          <input
            id={checkboxId}
            type="checkbox"
            checked={dontShow()}
            onChange={(e) => setDontShow(e.currentTarget.checked)}
          />
          Don't show this again
        </label>
      </div>
    </div>
  );
}
