import type { PageTarget, Route } from "./routeTypes";
import type { PageKind } from "./types";

type RouterBridge = {
  route(): Route;
  focusBlock(id: string | null): void;
  scheduleSessionSave(): void;
  openPageTarget(target: PageTarget): void;
  viewOwnerPath(name: string, pageKind: PageKind, path: string | undefined): string | undefined;
};

let bridge: RouterBridge | null = null;
export function installRouterBridge(next: RouterBridge): void { bridge = next; }
export function route(): Route { return bridge?.route() ?? { kind: "journals" }; }
export function focusBlock(id: string | null): void { bridge?.focusBlock(id); }
export function scheduleSessionSave(): void { bridge?.scheduleSessionSave(); }
export function openPageTarget(target: PageTarget): void { bridge?.openPageTarget(target); }
/** The router's one answer to "which file does this view show" (see `router.ts`). */
export function viewOwnerPath(name: string, pageKind: PageKind, path: string | undefined): string | undefined {
  return bridge ? bridge.viewOwnerPath(name, pageKind, path) : path;
}
