import type { Route } from "./routeTypes";

type RouterBridge = {
  route(): Route;
  focusBlock(id: string | null): void;
  scheduleSessionSave(): void;
};

let bridge: RouterBridge | null = null;
export function installRouterBridge(next: RouterBridge): void { bridge = next; }
export function route(): Route { return bridge?.route() ?? { kind: "journals" }; }
export function focusBlock(id: string | null): void { bridge?.focusBlock(id); }
export function scheduleSessionSave(): void { bridge?.scheduleSessionSave(); }
