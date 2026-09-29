// Settings → Help & diagnostics: the last error messages of this session,
// newest first, each copyable. In memory only (src/errorToastHistory.ts); it is
// not part of the diagnostic report because messages can name pages.
import { For, Show, type JSX } from "solid-js";
import { writeClipboardText } from "../clipboard";
import { dbg } from "../debug";
import { errorToastHistory } from "../errorToastHistory";
import { pushToast } from "../toasts";

export function ErrorToastHistory(): JSX.Element {
  const copy = async (text: string) => {
    try {
      await writeClipboardText(text);
      pushToast("Error message copied", "success");
    } catch (error) {
      dbg(`error message copy failed: ${String(error)}`);
      pushToast("Could not copy the error message.", "error");
    }
  };

  return (
    <div class="diagnostics-errors">
      <h3>Recent error messages</h3>
      <p class="settings-hint">
        The last 20 red error messages of this session, newest first. They are kept in memory only,
        are not part of the diagnostic report above (they can name pages), and are gone when Tine closes.
      </p>
      <Show when={errorToastHistory().length > 0} fallback={<p class="settings-hint">No error messages this session.</p>}>
        <ul class="diagnostics-error-list">
          <For each={errorToastHistory()}>
            {(entry) => (
              <li class="diagnostics-error-row">
                <time class="diagnostics-error-time" dateTime={new Date(entry.at).toISOString()}>
                  {new Date(entry.at).toLocaleTimeString()}
                </time>
                <span class="diagnostics-error-text">{entry.text}</span>
                <Show when={entry.count > 1}>
                  <span class="diagnostics-error-count">×{entry.count}</span>
                </Show>
                <button type="button" onClick={() => void copy(entry.text)}>Copy</button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
