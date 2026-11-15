import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal, Show } from "solid-js";
import { backend } from "./backend";
import { toolbarActions, saveToolbarChoice } from "./toolbarChoice";
import { createToolbarLayout } from "./toolbarLayout";
import { mainPaneRouter, resetTabsToJournals } from "./router";

let dispose = () => {};
afterEach(async () => { dispose(); resetTabsToJournals(); saveToolbarChoice(null); await Promise.resolve(); vi.unstubAllGlobals(); vi.restoreAllMocks(); document.documentElement.style.fontSize = ""; document.body.innerHTML = ""; });

it("measures once per resize, visits each button once, and disconnects; navigation does no work", () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  let resized!: ResizeObserverCallback;
  const observe = vi.fn(), disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class { constructor(callback: ResizeObserverCallback) { resized = callback; } observe = observe; disconnect = disconnect; });
  saveToolbarChoice({ bar: ["settings", "calendar", "journals", "theme", "share", "right-sidebar", "workspaces"], menu: [] });
  const host = document.body.appendChild(document.createElement("div"));
  const reads = vi.fn(() => 30);
  const width = vi.fn(() => 210);
  dispose = render(() => {
    const layout = createToolbarLayout(() => ({ available: toolbarActions.map(action => action.id), workspace: false, tabs: false, windowControls: false, navigationAtBottom: false }));
    return <><header class="topbar" ref={layout.ref}><div class="topbar-left" /><div class="topbar-spacer" /><div class="topbar-right">
      {toolbarActions.map(action => <button data-toolbar-action={action.id} ref={element => Object.defineProperty(element, "offsetWidth", { get: reads })} style={layout.style(action.id)} />)}
      <div class="topbar-overflow"><button /></div>
    </div></header><p>{mainPaneRouter.route().kind}</p></>;
  }, host);
  expect(observe).toHaveBeenCalledTimes(1);
  reads.mockClear();
  resized([{ contentRect: { get width() { return width(); } } }] as unknown as ResizeObserverEntry[], {} as ResizeObserver);
  expect(width).toHaveBeenCalledTimes(1);
  expect(reads).toHaveBeenCalledTimes(7);
  expect([...host.querySelectorAll<HTMLButtonElement>('[data-toolbar-action]')].filter(button => button.style.display !== "none").map(button => button.dataset.toolbarAction)).toContain("settings");
  mainPaneRouter.openPage("Tine", "page", { inPlace: true });
  expect(reads).toHaveBeenCalledTimes(7);
  dispose(); dispose = () => {};
  expect(disconnect).toHaveBeenCalledTimes(1);
});

it("carries tab-strip and window-control changes without measuring the container again", () => {
  document.documentElement.style.fontSize = "16px";
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  let resized!: ResizeObserverCallback;
  vi.stubGlobal("ResizeObserver", class { constructor(callback: ResizeObserverCallback) { resized = callback; } observe() {} disconnect() {} });
  const [tabs, setTabs] = createSignal(false), [controls, setControls] = createSignal(false);
  const host = document.body.appendChild(document.createElement("div"));
  dispose = render(() => {
    const layout = createToolbarLayout(() => ({ available: toolbarActions.map(action => action.id), workspace: false, tabs: tabs(), windowControls: controls(), navigationAtBottom: false }));
    return <header class="topbar" ref={layout.ref}><div class="topbar-left" /><Show when={tabs()}><div class="tab-bar" /></Show><div class="topbar-spacer" /><div class="topbar-right">
      {toolbarActions.map(action => <button data-toolbar-action={action.id} style={layout.style(action.id)} />)}
      <div class="topbar-overflow" /><Show when={controls()}><div class="win-controls" /></Show>
    </div></header>;
  }, host);
  const width = vi.fn(() => 400);
  resized([{ contentRect: { get width() { return width(); } } }] as unknown as ResizeObserverEntry[], {} as ResizeObserver);
  const visible = (id: string) => host.querySelector<HTMLElement>(`[data-toolbar-action="${id}"]`)!.style.display !== "none";
  expect(visible("calendar")).toBe(true);
  setTabs(true);
  expect(visible("calendar")).toBe(false);
  setTabs(false);
  expect(visible("calendar")).toBe(true);
  setControls(true);
  expect(visible("calendar")).toBe(false);
  expect(visible("right-sidebar")).toBe(false);
  expect(width).toHaveBeenCalledTimes(1);
});

it("reuses freed header space when navigation moves to the bottom", () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  let resized!: ResizeObserverCallback;
  vi.stubGlobal("ResizeObserver", class { constructor(callback: ResizeObserverCallback) { resized = callback; } observe() {} disconnect() {} });
  const [bottom, setBottom] = createSignal(false);
  saveToolbarChoice({ bar: ["settings", "theme", "calendar", "journals", "share"], menu: ["right-sidebar", "workspaces"] });
  const host = document.body.appendChild(document.createElement("div"));
  dispose = render(() => {
    const layout = createToolbarLayout(() => ({ available: toolbarActions.map(action => action.id), workspace: false, tabs: false, windowControls: false, navigationAtBottom: bottom() }));
    return <header class="topbar" ref={layout.ref}><Show when={!bottom()}><div class="topbar-left" ref={element => Object.defineProperty(element, "offsetWidth", { value: 90 })} /></Show><div class="topbar-spacer" /><div class="topbar-right">
      {toolbarActions.map(action => <button data-toolbar-action={action.id} ref={element => Object.defineProperty(element, "offsetWidth", { value: 30 })} style={layout.style(action.id)} />)}
      <div class="topbar-overflow" />
    </div></header>;
  }, host);
  const width = vi.fn(() => 210);
  resized([{ contentRect: { get width() { return width(); } } }] as unknown as ResizeObserverEntry[], {} as ResizeObserver);
  const share = host.querySelector<HTMLElement>('[data-toolbar-action="share"]')!;
  expect(share.style.display).toBe("none");
  setBottom(true);
  expect(share.style.display).not.toBe("none");
  expect(width).toHaveBeenCalledTimes(1);
});
