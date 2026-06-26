import { createSignal, type JSX } from "solid-js";
import { appendToTodayJournal, captureToPage } from "../store";
import { pushToast } from "../ui";

// In-app quick capture, pinned to the top of the journal feed. A page-title-styled
// field + a body composer: submit (Ctrl/Cmd-Shift-Enter) routes the body to a NEW
// (or existing) page when the title is filled, else appends it to the end of
// today's journal. The body is plain multi-line text turned into an outline on
// submit (one line → one block; leading 2-space / tab indent → nesting, via the
// same parseOutline the global capture window uses), so it round-trips like any
// other captured note. Both writes go through the live store's single-writer
// append (captureToPage / appendToTodayJournal) — no scratch page is ever left in
// the working set, so nothing can leak into the feed, All-Pages, or persistence.
export function QuickCapture(): JSX.Element {
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  let bodyRef!: HTMLTextAreaElement;

  const grow = () => {
    bodyRef.style.height = "auto";
    bodyRef.style.height = `${bodyRef.scrollHeight}px`;
  };

  const reset = () => {
    setTitle("");
    setBody("");
    queueMicrotask(grow);
  };

  const submit = async () => {
    if (busy()) return;
    const t = title().trim();
    const b = body().trim();
    if (!b) {
      pushToast("Nothing to capture — type something in the body first.", "info");
      bodyRef.focus();
      return;
    }
    setBusy(true);
    try {
      const ok = t ? await captureToPage(t, body()) : await appendToTodayJournal(body());
      if (ok) {
        pushToast(t ? `Captured to “${t}”` : "Captured to today’s journal", "info");
        reset();
      } else {
        pushToast("Couldn’t capture — please try again.", "error");
      }
    } finally {
      setBusy(false);
    }
  };

  // Ctrl/Cmd-Shift-Enter submits from either field. stopPropagation so the global
  // keybinding layer doesn't also act on it.
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && e.shiftKey && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      void submit();
    }
  };

  return (
    <div class="quick-capture" classList={{ busy: busy() }}>
      <input
        class="qc-title"
        type="text"
        value={title()}
        placeholder="Page Title (optional, if empty → appended to Today)"
        onInput={(e) => setTitle(e.currentTarget.value)}
        onKeyDown={onKey}
      />
      <textarea
        class="qc-body"
        ref={bodyRef}
        rows={1}
        value={body()}
        placeholder="Edit as usual, Ctrl-Shift-Enter to submit"
        onInput={(e) => {
          setBody(e.currentTarget.value);
          grow();
        }}
        onKeyDown={onKey}
      />
    </div>
  );
}
