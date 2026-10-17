import { t } from "../i18n";
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
import {
  inPageReplaceOpen,
  inPageReplaceText,
  replaceActiveInPageFindMatch,
  replaceAllInPageFindMatches,
  setInPageReplaceOpen,
  setInPageReplaceText,
} from "../inpageReplace";
import { dismissTopTransient, registerTransientLayer } from "../transientLayers";
import { onEachWindow } from "../windowRealm";

const FIND_QUERY_DEBOUNCE_MS = 110;

export function InPageFind(): JSX.Element {
  let root: HTMLDivElement | undefined;
  let inputEl: HTMLInputElement | undefined;
  let replaceEl: HTMLInputElement | undefined;
  let queryTimer: ReturnType<typeof setTimeout> | undefined;
  const [draftQuery, setDraftQuery] = createSignal(inPageFindQuery());
  const matchCount = () => inPageFindMatches().length;
  const countText = () => {
    const count = matchCount();
    if (count) return `${inPageFindActiveIndex() + 1} / ${count}`;
    return draftQuery().trim() && draftQuery() === inPageFindQuery() ? t("navigation-prose-no-results") : "";
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
  // GH #208: replace acts on the committed query, never on a debounced draft.
  const replaceOne = () => {
    if (queryTimer) commitQuery();
    replaceActiveInPageFindMatch(inPageReplaceText());
  };
  const replaceAll = () => {
    if (queryTimer) commitQuery();
    replaceAllInPageFindMatches(inPageReplaceText());
  };
  const toggleReplace = () => {
    const open = !inPageReplaceOpen();
    setInPageReplaceOpen(open);
    queueMicrotask(() => (open ? replaceEl : inputEl)?.focus());
  };
  const escapeKey = (e: KeyboardEvent) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (dismissTopTransient("escape")) e.preventDefault();
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
    // Highlights can sit in any Tine window's panes (OG-MULTIWINDOW P1).
    const stopWindows = onEachWindow((win) => {
      win.addEventListener("scroll", refresh, true);
      win.addEventListener("resize", refresh);
      return () => {
        win.removeEventListener("scroll", refresh, true);
        win.removeEventListener("resize", refresh);
      };
    });
    onCleanup(() => {
      stopWindows();
      cancelScheduledQuery();
    });
  });

  return (
    <Show when={inPageFindOpen()}>
      <div ref={root} class="inpage-find-bar" role="search">
        <div class="inpage-find-row">
          <input
            ref={inputEl}
            class="inpage-find-input"
            placeholder={t("in-page-find-find-in-page")}
            value={draftQuery()}
            onInput={(e) => scheduleQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (queryTimer) commitQuery();
                stepInPageFind(e.shiftKey ? -1 : 1);
              } else if (e.key === "Escape") {
                escapeKey(e);
              }
            }}
          />
          {/* After the input in focus order (a dismissed layer above Find refocuses
              Find's first control); CSS shows it first. */}
          <button
            type="button"
            class="icon-btn inpage-replace-toggle"
            title={t("in-page-find-find-and-replace")}
            aria-label={t("in-page-find-find-and-replace")}
            aria-expanded={inPageReplaceOpen()}
            onClick={toggleReplace}
          >
            <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
              <path d={inPageReplaceOpen() ? "M6 9l6 6 6-6" : "M9 6l6 6-6 6"} fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          </button>
          <span class="inpage-find-count">{countText()}</span>
          <button
            class="icon-btn"
            title={t("in-page-find-previous-match-shift-enter")}
            disabled={!matchCount()}
            onClick={() => stepInPageFind(-1)}
          >
            <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
              <path d="M6 15l6-6 6 6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          </button>
          <button
            class="icon-btn"
            title={t("in-page-find-next-match-enter")}
            disabled={!matchCount()}
            onClick={() => stepInPageFind(1)}
          >
            <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
              <path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          </button>
          <button class="icon-btn" title={t("in-page-find-close-esc")} onClick={closeFind}>
            <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
              <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
            </svg>
          </button>
        </div>
        <Show when={inPageReplaceOpen()}>
          <div class="inpage-find-row inpage-replace-row">
            <input
              ref={replaceEl}
              class="inpage-replace-input"
              placeholder={t("in-page-find-replace-with")}
              aria-label={t("in-page-find-replace-with")}
              value={inPageReplaceText()}
              onInput={(e) => setInPageReplaceText(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  if (e.isComposing || e.keyCode === 229) return;
                  e.preventDefault();
                  if (e.ctrlKey || e.metaKey) replaceAll();
                  else replaceOne();
                } else if (e.key === "Escape") {
                  escapeKey(e);
                }
              }}
            />
            <button type="button" class="inpage-replace-btn" title={t("in-page-find-replace-this-match-enter")} disabled={!matchCount()} onClick={replaceOne}>{t("in-page-find-replace")}</button>
            <button type="button" class="inpage-replace-btn" title={t("in-page-find-replace-every-match-on-this-page-ctrl-cmd")} disabled={!matchCount()} onClick={replaceAll}>{t("in-page-find-replace-all")}</button>
          </div>
        </Show>
      </div>
    </Show>
  );
}
