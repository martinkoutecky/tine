import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { captureBinding, invalidateBinding } from "../binding";
import { applyGraphConfigChange } from "../graph";
import { graphConfigUnreadable, setGraphConfigProblem } from "../graphConfig";
import { graphMeta, setGraphMeta } from "../graphSession";
import { blockPageReadOnly, pageByName, resetStore, setRaw, node } from "../document";
import { loadSingle } from "../document/workingSet";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { closeSettings, openSettings, setRightSidebar } from "../ui";
import { setToasts, toasts } from "../toasts";
import { PageView } from "./Page";
import { Settings } from "./Settings";
import { Toasts } from "./Toasts";
import { t } from "../i18n";
import type { PageRead } from "../types";

const problem = { kind: "config-read" as const, message: "invalid utf-8 sequence" };
const dto: PageRead = { id: "pages/Witness.md", name: "Witness", kind: "page", title: "Witness", pre_block: null,
  blocks: [{ id: "witness-block", raw: "Config recovery witness", collapsed: false, children: [] }], read_only: false, guide: false };

afterEach(() => {
  closeSettings(); setRightSidebar([]); resetStore(); resetTabsToJournals();
  setGraphMeta(null); setGraphConfigProblem(null); setToasts([]); vi.restoreAllMocks(); document.body.replaceChildren();
});

async function mount(readOnly = false, format: "md" | "org" = "md", configProblem: typeof problem | null = null) {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/config-witness" });
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(41);
  setGraphConfigProblem(configProblem);
  const page = { ...dto, format, id: `pages/Witness.${format}`, read_only: readOnly };
  const read = vi.spyOn(backend(), "getPage").mockResolvedValue(page);
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page);
  loadSingle(page);
  mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><PageView /><Settings /><Toasts /></>, root);
  await vi.waitFor(() => expect(root.querySelector("h1.page-title")?.textContent).toContain(dto.name));
  read.mockClear();
  return { root, dispose, read };
}

function publish(config_problem: typeof problem | null, generation = captureBinding().backendGeneration, root?: string) {
  applyGraphConfigChange({ binding_generation: generation, meta: { ...graphMeta()!, ...(root ? { root } : {}) }, config_problem });
}

it.each(["md", "org"] as const)("Hl2_0: an open-time config restriction names config.edn on a routed %s page", async format => {
  const view = await mount(true, format, problem);
  try {
    expect(view.root.querySelector(".page-readonly-banner")?.textContent).toBe(t("ui-failure-config-read"));
    expect(view.root.querySelector(".page-readonly-banner")?.textContent).not.toContain(".org");
  } finally { view.dispose(); }
});

it("Hl2_0: live failure shows one notice, blocks editing everywhere, and repair clears it without fetching pages", async () => {
  const view = await mount();
  const before = pageByName(dto.name);
  try {
    expect(blockPageReadOnly("witness-block")).toBe(false);
    publish(problem, 42);
    expect(view.root.querySelector(".page-readonly-banner")).toBeNull();
    publish(problem);
    await vi.waitFor(() => expect(view.root.querySelector(".page-readonly-banner")?.textContent).toBe(t("ui-failure-config-read")));
    expect(blockPageReadOnly("witness-block")).toBe(true);
    setRaw("witness-block", "refused edit");
    expect(node("witness-block").raw).toBe("Config recovery witness");
    expect(view.root.querySelector(".page-trailing-block-target")).toBeNull();
    expect(view.root.querySelector(".toast-error .toast-msg")?.textContent).toBe(t("ui-failure-config-read"));
    publish(problem);
    expect(toasts().filter(toast => toast.text.kind === "msg" && toast.text.id === "ui-failure-config-read")).toHaveLength(1);
    expect(toasts().find(toast => toast.message.includes("config.edn"))?.count ?? 1).toBe(1);
    openSettings("appearance");
    expect(view.root.querySelector('.settings-modal [role="alert"]')?.textContent).toContain("config.edn");
    publish(null, 42);
    publish(null, 41, "/another-graph");
    expect(graphConfigUnreadable()).toBe(true);
    publish(null);
    await vi.waitFor(() => expect(view.root.querySelector(".page-readonly-banner")).toBeNull());
    expect(view.root.querySelector('.settings-modal [role="alert"]')).toBeNull();
    expect(view.root.querySelector(".toast-error")).toBeNull();
    expect(blockPageReadOnly("witness-block")).toBe(false);
    expect(pageByName(dto.name)).toBe(before);
    expect(view.read).not.toHaveBeenCalled();
    setRaw("witness-block", "recovered edit");
    expect(node("witness-block").raw).toBe("recovered edit");
  } finally { view.dispose(); }
});

it("Hl2_0: retiring the graph binding clears its restriction and notice", async () => {
  const view = await mount();
  try {
    publish(problem);
    expect(graphConfigUnreadable()).toBe(true);
    invalidateBinding();
    expect(graphConfigUnreadable()).toBe(false);
    expect(view.root.querySelector(".page-readonly-banner")).toBeNull();
    expect(view.root.querySelector(".toast-error")).toBeNull();
  } finally { view.dispose(); }
});

it("Hl2_0: an intrinsic Org restriction survives config repair with the Org explanation", async () => {
  const view = await mount(true, "org");
  try {
    publish(problem);
    publish(null);
    expect(view.root.querySelector(".page-readonly-banner")?.textContent).toContain(".org");
    expect(blockPageReadOnly("witness-block")).toBe(true);
  } finally { view.dispose(); }
});
