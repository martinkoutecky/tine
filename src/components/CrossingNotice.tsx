import { Show, createSignal, createUniqueId, onMount, type JSX } from "solid-js";

/** **The user half of the persisted-form crossing (SPEC §4.3 "Notice", §7.5).**
*
*  The mechanical half already shipped: an edit the OG DSL cannot express turns
*  a `{{query …}}` block into `{{tine-query …}}` on save. That is the right
*  behaviour — the alternative is dropping the user's edit — but it changes what
*  the block looks like *in Logseq*, silently, as a side effect of an ordinary
*  edit. So it is said out loud, with the way back attached.
*
*  Three deliberate shapes:
*
*  1. **Not a toast.** P4 hosts this same component inside the query pane it
*     builds, so it is an inline region that takes its position from its host.
*     A toast would also be gone before a reader who looked away came back.
*  2. **The feature is NOT named in P2.** The engine's answer here is a bare
*     `og_expressible` bool, and the OG printer's refusal says only "this
*     filter" / "this view". Re-deriving OG-expressibility over the TypeScript
*     IR mirror to produce a name would be a second producer of a question Rust
*     owns (D-14) — the exact class of twin this campaign deleted. `feature` is
*     the seam P4 fills from the pane's diagnostics without the text moving.
*  3. **Undo is the ordinary undo**, gated on the ordinary undo actually being
*     this change (`canUndo`). There is no targeted-inverse mechanism: an undo
*     that reached past a later edit to reverse an older one would be a new and
*     much more dangerous kind of undo.
*/
export function CrossingNotice(props: {
  /** Whether the change this notice offers to take back is still the change the ordinary Undo would take back. */
  canUndo: boolean;
  /** The §4.3 seam for NAMING the unsupported feature. */
  feature?: string;
  /** **A bounded excerpt of the text the engine just printed (P4).**
  *
  *  Not the same claim as `feature`, which is why it is not the same prop and
  *  not the same sentence: this says "here is what the block now says", which
  *  is evidence the save path already has, rather than "here is the feature
  *  that crossed", which nothing today can honestly answer. */
  changed?: string;
  onUndo: () => void;
  onKeep: () => void;
  /** Called at most once, on close, when "Don't show this again" is ticked. */
  onDontShowAgain: () => void;
  /** **The checkbox, optionally lifted (P4, N3).** P4 moves this one notice
  *  between two hosts — inline under the block, and inside the query text pane
  *  while the sheet is open — and a re-parented component is a re-created one.
  *  A host that owns the tick passes it down so the move cannot silently
  *  untick it; a host that does not keeps the local state P2 had. */
  dontShow?: boolean;
  onDontShowChange?: (value: boolean) => void;
    /** Takes focus on mount unless explicitly false. */
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
