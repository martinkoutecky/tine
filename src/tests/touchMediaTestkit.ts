// A stand-in for the platform's `matchMedia` whose answer a test can flip after
// mount, so the REAL `inputCapability.ts` signal chain (one `change` listener
// per query, a revision bump) is what a flip exercises. One list object per
// query for the life of the test file: inputCapability registers its single
// `change` listener on the first object it is handed.
type Listener = () => void;
interface FakeList { matches: boolean; listeners: Set<Listener>; media: string; }

// A plain record, not a module-level Map: test state, not graph state.
const lists: Record<string, FakeList> = {};

function listOf(query: string): FakeList {
  return lists[query] ??= { matches: false, listeners: new Set(), media: query };
}

/** Install the fake on `window` (jsdom has none). Call once per test file. */
export function installTouchMedia(): void {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) => {
      const list = listOf(query);
      return {
        get matches() { return list.matches; },
        media: query,
        addEventListener: (_: string, fn: Listener) => { list.listeners.add(fn); },
        removeEventListener: (_: string, fn: Listener) => { list.listeners.delete(fn); },
      };
    },
  });
}

/** Set what the platform answers and tell every listener, as a real change
 *  (a keyboard dock/undock, a convertible changing mode) does. `anyPointer`
 *  defaults to the primary answer. */
export function setTouchInput(primary: boolean, anyPointer = primary): void {
  for (const [query, value] of [["(pointer: coarse)", primary], ["(any-pointer: coarse)", anyPointer]] as const) {
    const list = listOf(query);
    if (list.matches === value) continue;
    list.matches = value;
    for (const fn of [...list.listeners]) fn();
  }
}
