// Point the session at the application window, not at an auxiliary one.
//
// A WebDriver session starts on whichever window the driver attached to. On
// Windows that is not reliably the main app: `windows-smoke`'s `pdf-logseq`
// spent its whole 20 s budget hunting `.pdf-link` inside a **Quick Capture**
// window (`http://tauri.localhost/capture.html`), and reported the missing
// link rather than the wrong window -- a failure that reads as a product defect
// and is not one.
//
// Tine's auxiliary windows are identified by the document they load, which is
// the app's own contract (src-tauri window configuration), not a heuristic
// about titles.
const AUXILIARY_DOCUMENTS = ["capture.html", "about.html"];

/** True when this URL is one of Tine's auxiliary windows rather than the app. */
export function isAuxiliaryWindow(url) {
  const path = String(url ?? "").split(/[?#]/)[0];
  return AUXILIARY_DOCUMENTS.some((document) => path.endsWith(`/${document}`));
}

/**
 * Choose the application window from an enumerated session.
 *
 * `windows` is `[{ handle, url }]`. Returns the handle to switch to, or null
 * when the session is already on an application window.
 */
export function chooseMainWindow(windows, currentHandle) {
  const current = windows.find((window) => window.handle === currentHandle);
  if (current && !isAuxiliaryWindow(current.url)) return null;
  const main = windows.find((window) => !isAuxiliaryWindow(window.url));
  return main ? main.handle : undefined;
}

/**
 * Switch the session to the application window if it is not already there.
 *
 * Throws naming every window it saw when there is no application window at
 * all, because "the app never opened" and "we are looking at the wrong window"
 * are different failures and only one of them is a product defect.
 */
export async function ensureMainWindow(browser, { what = "the application window" } = {}) {
  const handles = await browser.getWindowHandles();
  const current = await browser.getWindowHandle();
  const windows = [];
  for (const handle of handles) {
    await browser.switchToWindow(handle);
    windows.push({ handle, url: await browser.getUrl() });
  }
  const target = chooseMainWindow(windows, current);
  if (target === undefined) {
    throw new Error(
      `${what} is not among the session's windows: `
      + `${windows.map((window) => `${window.handle}=${window.url}`).join(", ")}`,
    );
  }
  await browser.switchToWindow(target ?? current);
}
