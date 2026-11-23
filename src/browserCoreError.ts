/** The browser could not initialize the graph engine. Its own module, so the
 * readers that only recognize it do not import the core's loader. */
export class BrowserCoreLoadError extends Error {
  constructor(cause: unknown) {
    super(`Browser graph engine unavailable: ${String(cause)}`, { cause });
  }
}
