import { beforeAll } from "vitest";

// The browser mock is a lazy chunk of the app (ADR 0081). Load its module (not
// the browser core) before each file's tests, so a test that reads a backend
// answer after a tick sees it, as when the mock was imported statically.
beforeAll(async () => {
  // Import after test modules register their own mocks.
  const { loadBrowserMock } = await import("./browserMock");
  await loadBrowserMock();
});
