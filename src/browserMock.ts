/** How `backend()` reaches the browser mock (ADR 0081): through one dynamic
 * import, only outside Tauri and outside a published export, and only in a
 * build that carries it (the dev server, the tests and `npm run build:mock`).
 * The desktop production bundle contains neither the mock nor the core. */
import type { Backend } from "./backendInterface";

/** Backend members the browser mock leaves absent: optional native-only
 * surfaces, so the browser fallbacks in `backend.ts` take over. */
export const MOCK_ABSENT_METHODS = new Set([
  "orphanDrafts", "recoverOrphanDrafts", "tineLinks", "ownerProtectionStatus", "retryOwnerProtection",
  "registerUnlinkedReference", "registerQuerySubscription", "pdfAssetSource", "readExternalPdf",
  "onOwnerFlushRequest", "onOwnerPageRenamed", "ownerFlushAnswer", "onGraphChangedBulk", "onGraphWatchStatus",
  "onGraphRescanComplete", "rescanGraphNow", "deleteOldBackups", "diagnosticTimingEvent",
]);

/** Whether this build carries the browser mock: a compile-time answer, so a
 * production build drops the mock's import and everything behind it. */
export const BROWSER_MOCK_BUILD = import.meta.env.MODE === "development" || import.meta.env.MODE === "test"
  || import.meta.env.MODE === "mock";

type MockModule = typeof import("./mock");
let mockModule: Promise<MockModule> | undefined;
let loadedModule: MockModule | undefined;

/** Load the mock module (not the core) once. The tests' setup awaits it, so a
 * backend call answers within a few microtasks, as from a loaded module. */
export function loadBrowserMock(): Promise<MockModule> {
  if (!BROWSER_MOCK_BUILD) {
    throw new Error("This build has no browser backend: open it in the Tine app or as a published export.");
  }
  return (mockModule ??= import("./mock").then(module => (loadedModule = module)));
}

type Members = Record<string, unknown>;

/** A backend whose every method waits for the mock module, loaded once. Once
 * the module is loaded it is the mock itself: its members are listed and read
 * directly, so a caller sees the same object as from a static import. */
export function browserMockBackend(): Backend {
  let instance: Members | undefined;
  const loading = loadBrowserMock();
  const loaded = () => (loadedModule ? (instance ??= loadedModule.mockBackend() as unknown as Members) : undefined);
  const mock = () => loading.then(() => loaded() as Members);
  return new Proxy({} as Members, {
    get(target, property) {
      if (typeof property !== "string") return undefined;
      if (property in target) return target[property];
      if (property === "then" || MOCK_ABSENT_METHODS.has(property)) return undefined;
      const ready = loaded();
      if (ready) return ready[property];
      // The one synchronous member: the mock binds one graph for its lifetime.
      if (property === "graphBindingGeneration") return () => 1;
      return (...args: unknown[]) => mock().then(backend =>
        (backend[property] as (...args: unknown[]) => unknown)(...args));
    },
    // An assignment overrides the member, as on a plain object (a test's fake).
    set(target, property, value) {
      return Reflect.defineProperty(target, property, { value, writable: true, enumerable: true, configurable: true });
    },
    has(target, property) {
      return typeof property === "string" && !MOCK_ABSENT_METHODS.has(property) ? true : property in target;
    },
    ownKeys(target) {
      const ready = loaded();
      const keys = ready ? Object.keys(ready).filter(key => !MOCK_ABSENT_METHODS.has(key)) : [];
      return [...new Set([...keys, ...Reflect.ownKeys(target)])];
    },
    getOwnPropertyDescriptor(target, property) {
      const own = Reflect.getOwnPropertyDescriptor(target, property);
      if (own || typeof property !== "string" || MOCK_ABSENT_METHODS.has(property)) return own;
      const ready = loaded();
      return ready && property in ready
        ? { value: ready[property], writable: true, enumerable: true, configurable: true }
        : undefined;
    },
  }) as unknown as Backend;
}
