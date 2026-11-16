import { afterEach, expect, it, vi } from "vitest";
import { setActiveLocale } from "./i18n";
const { invoke } = vi.hoisted(() => ({ invoke:vi.fn() }));
vi.mock("@tauri-apps/api/core",()=>({ invoke,convertFileSrc:(path:string)=>path }));
afterEach(async()=>{ await setActiveLocale("en"); vi.unstubAllGlobals(); invoke.mockReset(); });
it("both graph-open doors translate the shared app-owner refusal without changing the binding",async()=>{
  vi.stubGlobal("window",{ __TAURI_INTERNALS__:{} });
  invoke.mockImplementation((command:string)=> command === "load_graph" || command === "open_graph_window"
    ? Promise.reject("graph-app-owner") : Promise.resolve(null));
  const { backend } = await import("./backend");
  const api=backend();
  for (const [locale,message] of [["en","The graph is open in another Tine window."],["zh-CN","此图谱已在另一个 Tine 窗口中打开。"]]) {
    await setActiveLocale(locale);
    await expect(api.loadGraph("/graph")).rejects.toThrow(message);
    await expect(api.openGraphWindow("/graph")).rejects.toThrow(message);
    expect(api.graphBindingGeneration()).toBe(0);
  }
});
