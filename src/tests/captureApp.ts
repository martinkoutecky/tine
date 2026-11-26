// Mount the Quick Capture app in a test file and end the mount before jsdom is
// torn down. capture.tsx renders itself at import and drops the dispose handle,
// because its WebView owns the root for life. In a test file the app would
// outlive the DOM: work still running when the file ends (a settings read, a
// startup listener) would then render its toast into a document that no longer
// exists, an unhandled "document is not defined" that fails the whole run.
// Importing this module keeps the handle; the file's afterAll disposes the app,
// so a later toast only changes the toast store.
import { afterAll, vi } from "vitest";

const h = vi.hoisted(() => ({ dispose: undefined as undefined | (() => void) }));

vi.mock("solid-js/web", async (original) => {
  const web = await original<typeof import("solid-js/web")>();
  return {
    ...web,
    render: ((code, element, init) => {
      const dispose = web.render(code, element, init);
      if ((element as Element).id === "capture-root") h.dispose = dispose;
      return dispose;
    }) as typeof web.render,
  };
});

/** Add `#capture-root` and import the Capture entry point, which mounts into it. */
export async function mountCapture(): Promise<HTMLElement> {
  const root = document.createElement("div");
  root.id = "capture-root";
  document.body.append(root);
  await import("../capture");
  return root;
}

/** End the Capture mount now (the file's afterAll also does). Idempotent. */
export function disposeCapture(): void {
  h.dispose?.();
  h.dispose = undefined;
}

afterAll(disposeCapture);
