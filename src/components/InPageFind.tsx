import { Show, createEffect, onCleanup, onMount, type JSX } from "solid-js";
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

export function InPageFind(): JSX.Element {
  let inputEl: HTMLInputElement | undefined;
  const matchCount = () => inPageFindMatches().length;
  const countText = () => {
    const count = matchCount();
    if (count) return `${inPageFindActiveIndex() + 1} / ${count}`;
    return inPageFindQuery().trim() ? "No results" : "";
  };

  createEffect(() => {
    inPageFindOpen();
    inPageFindQuery();
    inPageFindActiveIndex();
    queueMicrotask(refreshInPageFindHighlights);
  });

  createEffect(() => {
    inPageFindFocusRequest();
    if (!inPageFindOpen()) return;
    queueMicrotask(() => {
      inputEl?.focus();
      inputEl?.select();
    });
  });

  onMount(() => {
    const refresh = () => refreshInPageFindHighlights();
    window.addEventListener("scroll", refresh, true);
    window.addEventListener("resize", refresh);
    onCleanup(() => {
      window.removeEventListener("scroll", refresh, true);
      window.removeEventListener("resize", refresh);
    });
  });

  return (
    <Show when={inPageFindOpen()}>
      <div class="inpage-find-bar" role="search">
        <input
          ref={inputEl}
          class="inpage-find-input"
          placeholder="Find in page"
          value={inPageFindQuery()}
          onInput={(e) => setInPageFindQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              stepInPageFind(e.shiftKey ? -1 : 1);
            } else if (e.key === "Escape") {
              e.preventDefault();
              closeInPageFind();
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
        <button class="icon-btn" title="Close (Esc)" onClick={() => closeInPageFind()}>
          <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </Show>
  );
}

