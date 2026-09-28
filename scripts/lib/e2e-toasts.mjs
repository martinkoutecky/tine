// Error toasts shown while the app starts. A transient toast lives 3.2 s
// (src/toasts.ts), so polling the DOM every 150 ms from session creation sees
// every one that appears in the window. Returns the distinct error texts.
export async function watchErrorToasts(browser, ms = 4000) {
  const seen = new Set();
  const until = Date.now() + ms;
  while (Date.now() < until) {
    let texts = [];
    try {
      texts = await browser.execute(() =>
        [...document.querySelectorAll(".toast.toast-error")].map((node) => (node.textContent ?? "").replace(/×$/, "").trim()));
    } catch {
      // The document may be mid-navigation during startup; the next poll retries.
    }
    for (const text of texts) seen.add(text);
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  return [...seen];
}

// og 12e P2: a window that has not bound its graph yet reads no feed and
// reports no failure. Any feed-load error toast at startup is a false alarm.
export const FEED_LOAD_FAILURE = "Could not load journal feed";
