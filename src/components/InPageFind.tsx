import { Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";
import {
  closeInPageFind,
  inPageFindActiveIndex,
  inPageFindFocusRequest,
  inPageFindMatches,
  inPageFindOpen,
  inPageFindQuery,
  refreshInPageFindHighlights,
  setInPageFindQuery,
  stepInPageFind,
} from "../inpageFind";
import { dismissTopTransient, registerTransientLayer } from "../transientLayers";

const FIND_QUERY_DEBOUNCE_MS = 110;

export function InPageFind(): JSX.Element {
  let root: HTMLDivElement | undefined;
  let inputEl: HTMLInputElement | undefined;
  let queryTimer: ReturnType<typeof setTimeout> | undefined;
  const [draftQuery, setDraftQuery] = createSignal(inPageFindQuery());
  const matchCount = () => inPageFindMatches().length;
  const countText = () => {
    const count = matchCount();
    if (count) return `${inPageFindActiveIndex() + 1} / ${count}`;
    return draftQuery().trim() && draftQuery() === inPageFindQuery() ? "No results" : "";
  };
  const cancelScheduledQuery = () => {
    clearTimeout(queryTimer);
    queryTimer = undefined;
  };
  const commitQuery = (q = draftQuery()) => {
    cancelScheduledQuery();
    setInPageFindQuery(q);
  };
  const scheduleQuery = (q: string) => {
    setDraftQuery(q);
    cancelScheduledQuery();
    queryTimer = setTimeout(() => commitQuery(q), FIND_QUERY_DEBOUNCE_MS);
  };
  const closeFind = () => {
    cancelScheduledQuery();
    closeInPageFind();
  };

  createEffect(() => {
    inPageFindOpen();
    inPageFindQuery();
    inPageFindActiveIndex();
    queueMicrotask(refreshInPageFindHighlights);
  });
  createEffect(() => {
    if (!inPageFindOpen()) return;
    const unregister = registerTransientLayer({
      id: "in-page-find",
      root: () => root ?? null,
      dismiss: () => { closeFind(); return true; },
    });
    onCleanup(unregister);
  });

  createEffect(on(inPageFindFocusRequest, () => {
    if (!inPageFindOpen()) return;
    setDraftQuery(inPageFindQuery());
    queueMicrotask(() => {
      inputEl?.focus();
      inputEl?.select();
    });
  }));

  createEffect(() => {
    if (inPageFindOpen()) return;
    cancelScheduledQuery();
    setDraftQuery(inPageFindQuery());
  });

  onMount(() => {
    const refresh = () => refreshInPageFindHighlights();
    window.addEventListener("scroll", refresh, true);
    window.addEventListener("resize", refresh);
    onCleanup(() => {
      window.removeEventListener("scroll", refresh, true);
      window.removeEventListener("resize", refresh);
      cancelScheduledQuery();
    });
  });

  return (
    <Show when={inPageFindOpen()}>
      <div ref={root} class="inpage-find-bar" role="search">
        <input
          ref={inputEl}
          class="inpage-find-input"
          placeholder="Find in page"
          value={draftQuery()}
          onInput={(e) => scheduleQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              if (queryTimer) commitQuery();
              stepInPageFind(e.shiftKey ? -1 : 1);
            } else if (e.key === "Escape") {
              if (e.isComposing || e.keyCode === 229) return;
              if (dismissTopTransient("escape")) e.preventDefault();
            }
          }}
        />
        <span class="inpage-find-count">{countText()}</span>
        <button
          class="icon-btn"
          title="Previous match (Shift+Enter)"
          disabled={!matchCount()}
          onClick={() => stepInPageFind(-1)}
        >
          <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
            <path d="M6 15l6-6 6 6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          class="icon-btn"
          title="Next match (Enter)"
          disabled={!matchCount()}
          onClick={() => stepInPageFind(1)}
        >
          <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
            <path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button class="icon-btn" title="Close (Esc)" onClick={closeFind}>
          <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </Show>
  );
}
