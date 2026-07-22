/** One dismissal stack for transient UI.  Registration order, rather than DOM
 * listener order, is the authority for Escape and Android Back. */
export type TransientDismissReason = "escape" | "back" | "explicit";
export interface TransientLayer {
  id: string;
  /** Semantic parent for branch-local dismissal order; unrelated to DOM containment. */
  parentId?: string;
  root?: () => HTMLElement | null;
  trigger?: () => HTMLElement | null;
  dismiss: (reason: TransientDismissReason) => boolean;
}

type Entry = TransientLayer & { token: number };
let serial = 0;
const layers = new Map<string, Entry>();

function containsEventTarget(root: HTMLElement, target: EventTarget | null): boolean {
  try {
    return target != null && root.contains(target as Node);
  } catch {
    return false;
  }
}

export function registerTransientLayer(layer: TransientLayer): () => void {
  // A stale Solid cleanup must never remove a newer owner that reused the
  // stable semantic id (for example a completion remounting while its prior
  // effect is being disposed).  Keep the entry identity in the disposer.
  const entry: Entry = { ...layer, token: ++serial };
  layers.set(layer.id, entry);
  // A visible owner can be brought in front of another visible owner without a
  // remount (for example two right-sidebar editors).  Bind this at the actual
  // registered root rather than relying on a registry-only test call.
  const activateFromEvent = (event: Event) => {
    const root = entry.root?.();
    const target = event.target;
    if (layers.get(entry.id) === entry && root && containsEventTarget(root, target)) {
      entry.token = ++serial;
    }
  };
  if (typeof document !== "undefined" && typeof document.addEventListener === "function") {
    document.addEventListener("focusin", activateFromEvent, true);
    document.addEventListener("pointerdown", activateFromEvent, true);
  }
  return () => {
    if (typeof document !== "undefined" && typeof document.removeEventListener === "function") {
      document.removeEventListener("focusin", activateFromEvent, true);
      document.removeEventListener("pointerdown", activateFromEvent, true);
    }
    if (layers.get(layer.id) === entry) layers.delete(layer.id);
  };
}

export function activateTransientLayer(id: string) {
  const layer = layers.get(id);
  if (layer) layer.token = ++serial;
}

export function topTransientLayer(): Entry | undefined {
  // Normalize the live parent graph before ranking it.  A missing parent is a
  // root; every member of a detected cycle becomes a root, while descendants of
  // those members retain their edge.  The resulting graph is a forest.
  const parent = new Map<string, string | undefined>();
  for (const entry of layers.values()) {
    parent.set(entry.id, entry.parentId && layers.has(entry.parentId) ? entry.parentId : undefined);
  }
  const visited = new Set<string>();
  for (const start of parent.keys()) {
    if (visited.has(start)) continue;
    const path: string[] = [];
    const position = new Map<string, number>();
    let current: string | undefined = start;
    while (current && !visited.has(current)) {
      const prior = position.get(current);
      if (prior != null) {
        for (const member of path.slice(prior)) parent.set(member, undefined);
        break;
      }
      position.set(current, path.length);
      path.push(current);
      current = parent.get(current);
    }
    for (const id of path) visited.add(id);
  }

  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const id of parent.keys()) {
    const parentId = parent.get(id);
    if (!parentId) roots.push(id);
    else (children.get(parentId) ?? children.set(parentId, []).get(parentId)!).push(id);
  }
  const subtreeMax = new Map<string, number>();
  const maximumToken = (id: string): number => {
    const cached = subtreeMax.get(id);
    if (cached != null) return cached;
    let maximum = layers.get(id)!.token;
    for (const child of children.get(id) ?? []) maximum = Math.max(maximum, maximumToken(child));
    subtreeMax.set(id, maximum);
    return maximum;
  };
  const greatest = (ids: string[]) => ids.reduce((best, id) => {
    const candidateToken = maximumToken(id);
    const bestToken = maximumToken(best);
    return candidateToken > bestToken || (candidateToken === bestToken && id > best) ? id : best;
  });

  if (!roots.length) return undefined;
  let id = greatest(roots);
  for (;;) {
    const directChildren = children.get(id) ?? [];
    if (!directChildren.length) return layers.get(id);
    id = greatest(directChildren);
  }
}

/** Test-only control for literal token-tie coverage of the public total order. */
export function setTransientLayerTokenForTest(id: string, token: number) {
  const layer = layers.get(id);
  if (!layer) return;
  layer.token = token;
  serial = Math.max(serial, token);
}

/** A false/stale top layer is removed but does not allow one physical gesture
 * to fall through and dismiss a lower layer. */
export function dismissTopTransient(reason: TransientDismissReason): boolean {
  const top = topTransientLayer();
  if (!top) return false;
  const handled = top.dismiss(reason);
  if (!handled && layers.get(top.id) === top) layers.delete(top.id);
  if (handled) queueMicrotask(() => restoreAfterTransientDismissal(top));
  return true;
}

function isFocusCandidate(candidate: HTMLElement): boolean {
  if (!candidate.isConnected) return false;
  for (let element: HTMLElement | null = candidate; element; element = element.parentElement) {
    if (element.hidden || element.getAttribute("aria-hidden") === "true" || element.inert || element.hasAttribute("inert")) return false;
  }
  if (candidate.matches(":disabled") || candidate.hasAttribute("disabled")) return false;
  return candidate.matches("button, input, select, textarea, a[href], [tabindex]");
}

/** Attempt only candidates that can own focus, and never mistake an ignored
 * focus call for success. */
function tryFocus(candidate: HTMLElement | null | undefined): boolean {
  if (!candidate || !isFocusCandidate(candidate)) return false;
  try {
    candidate.focus();
  } catch {
    return false;
  }
  return document.activeElement === candidate;
}

function tryFocusRoot(root: HTMLElement | null | undefined): boolean {
  if (!root?.isConnected) return false;
  for (const candidate of root.querySelectorAll<HTMLElement>("button, input, select, textarea, a[href], [tabindex]")) {
    if (tryFocus(candidate)) return true;
  }
  return tryFocus(root);
}

/** Restore to the exact dismissed opener first, then the surviving semantic
 * owner, and finally the active drawer.  The registry deliberately stays
 * independent of ui.ts. */
function restoreAfterTransientDismissal(top: Entry) {
  if (typeof document === "undefined") return;
  if (tryFocus(top.trigger?.())) return;
  const newer = topTransientLayer();
  if (newer && tryFocus(newer.trigger?.())) return;
  if (newer && tryFocusRoot(newer.root?.())) return;
  const drawer = document.querySelector<HTMLElement>("[data-active-drawer='left'] .left-sidebar, [data-active-drawer='right'] .right-sidebar");
  tryFocusRoot(drawer);
}

export function clearTransientLayersForTest() {
  layers.clear();
  serial = 0;
}
