/** Focus a field the moment it is mounted, optionally selecting its text.
 *
 * The `autofocus` attribute does not do this for a field inserted after the
 * page loaded: Chromium (WebView2 on Windows, Android's WebView) honours it
 * once per document, so the second rename or value editor needed a click
 * (GH #619), and WebKit does not do it reliably either (GH #176). Use it as
 * `ref={focusOnMount}`. */
export function focusOnMount(field: HTMLInputElement | HTMLTextAreaElement, select = false): void {
  queueMicrotask(() => {
    if (!field.isConnected) return;
    field.focus();
    if (select) field.select();
  });
}
