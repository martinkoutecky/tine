import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import {
  dispatchAndroidBack,
  exitAndroidActivity,
  installAndroidBackHandler,
  type AndroidBackDispatchDeps,
  type AndroidBackListener,
} from "./androidBack";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function dispatchDeps(): AndroidBackDispatchDeps & {
  transient: boolean;
  drawer: boolean;
  routerHasBack: boolean;
} {
  const state = {
    transient: false,
    drawer: false,
    routerHasBack: true,
    dismissTransient: vi.fn(() => state.transient),
    dismissDrawer: vi.fn(() => state.drawer),
    restoreDrawerFocus: vi.fn(),
    historyBack: vi.fn(() => state.routerHasBack),
    closeRoot: vi.fn(),
  };
  return state;
}

describe("GH #161 official Android AppPlugin Back owner", () => {
  it("peels exactly transient, drawer, one history step, then root close", () => {
    const deps = dispatchDeps();
    deps.transient = true;
    expect(dispatchAndroidBack({ canGoBack: true }, deps)).toBe("transient");
    expect(deps.dismissDrawer).not.toHaveBeenCalled();
    expect(deps.historyBack).not.toHaveBeenCalled();

    deps.transient = false;
    deps.drawer = true;
    expect(dispatchAndroidBack({ canGoBack: true }, deps)).toBe("drawer");
    expect(deps.restoreDrawerFocus).toHaveBeenCalledOnce();
    expect(deps.historyBack).not.toHaveBeenCalled();

    deps.drawer = false;
    expect(dispatchAndroidBack({ canGoBack: true }, deps)).toBe("history");
    expect(deps.historyBack).toHaveBeenCalledOnce();
    expect(deps.closeRoot).not.toHaveBeenCalled();

    deps.routerHasBack = false;
    expect(dispatchAndroidBack({ canGoBack: false }, deps)).toBe("root");
    expect(deps.historyBack).toHaveBeenCalledTimes(2);
    expect(deps.closeRoot).toHaveBeenCalledOnce();
  });

  // Master 393973956 -> 07cb27262: the WebView's canGoBack can be true while
  // Tine's router has nothing to pop. Choosing the history rung from it made
  // Back silently do nothing, forever; the router decides instead.
  it("closes the root when the router cannot go back even though the WebView says it can", () => {
    const deps = dispatchDeps();
    deps.routerHasBack = false;
    expect(dispatchAndroidBack({ canGoBack: true }, deps)).toBe("root");
    expect(deps.historyBack).toHaveBeenCalledOnce();
    expect(deps.closeRoot).toHaveBeenCalledOnce();
  });

  it("goes back through the router even when the WebView reports no history", () => {
    const deps = dispatchDeps();
    expect(dispatchAndroidBack({ canGoBack: false }, deps)).toBe("history");
    expect(deps.closeRoot).not.toHaveBeenCalled();
  });

  // GH #386: Tauri's plugin:app has no exit command, so the root close left a
  // gray, unusable screen. The installed process plugin exits.
  it("hands a safely prepared root close to Tauri's installed process exit API", async () => {
    const exit = vi.fn(async (_code?: number) => {});
    await exitAndroidActivity(async () => ({ exit }));
    expect(exit).toHaveBeenCalledWith(0);
  });

  it("wires App.tsx to the router rung and the process exit, with the exit permission granted", () => {
    const app = readFileSync("src/App.tsx", "utf8");
    const capability = JSON.parse(readFileSync("src-tauri/capabilities/default.json", "utf8")) as {
      permissions: string[];
    };
    expect(app).not.toContain("plugin:app|exit");
    expect(app).toContain("    exitAndroidActivity,\n");
    expect(app).toMatch(/historyBack: \(\) => \{\s*if \(!canGoBack\(\)\) return false;\s*goBack\(\);\s*return true;/);
    expect(capability.permissions).toContain("process:allow-exit");
  });

  it("subscribes exactly once only on Android and unregisters idempotently", async () => {
    const deps = dispatchDeps();
    const unregister = vi.fn(async () => {});
    let handler: ((payload: { canGoBack: boolean }) => void) | undefined;
    const subscribe = vi.fn(async (next) => {
      handler = next;
      return { unregister };
    });
    const uninstall = installAndroidBackHandler({
      ...deps,
      platform: async () => "android",
      subscribe,
    });
    await vi.waitFor(() => expect(subscribe).toHaveBeenCalledOnce());
    expect(handler).toBeTypeOf("function");
    handler!({ canGoBack: true });
    expect(deps.historyBack).toHaveBeenCalledOnce();

    uninstall();
    uninstall();
    expect(unregister).toHaveBeenCalledOnce();
  });

  it.each(["desktop", "ios"] as const)("does not subscribe on %s", async (platform) => {
    const deps = dispatchDeps();
    const subscribe = vi.fn();
    installAndroidBackHandler({ ...deps, platform: async () => platform, subscribe });
    await Promise.resolve();
    await Promise.resolve();
    expect(subscribe).not.toHaveBeenCalled();
  });

  it("leaves native fallback intact when platform or subscription setup rejects", async () => {
    for (const failure of ["platform", "subscribe"] as const) {
      const deps = dispatchDeps();
      const setupFailed = vi.fn();
      const subscribe = vi.fn(async () => {
        if (failure === "subscribe") throw new Error("subscription failed");
        return { unregister: vi.fn() };
      });
      installAndroidBackHandler({
        ...deps,
        platform: async () => {
          if (failure === "platform") throw new Error("platform failed");
          return "android";
        },
        subscribe,
        setupFailed,
      });
      await vi.waitFor(() => expect(setupFailed).toHaveBeenCalledOnce());
      expect(deps.historyBack).not.toHaveBeenCalled();
      expect(deps.closeRoot).not.toHaveBeenCalled();
    }
  });

  it("does not subscribe when cleanup wins the pending platform race", async () => {
    const platform = deferred<"android">();
    const deps = dispatchDeps();
    const subscribe = vi.fn();
    const uninstall = installAndroidBackHandler({ ...deps, platform: () => platform.promise, subscribe });
    uninstall();
    platform.resolve("android");
    await Promise.resolve();
    await Promise.resolve();
    expect(subscribe).not.toHaveBeenCalled();
  });

  it("unregisters immediately when cleanup wins the pending subscription race", async () => {
    const installed = deferred<AndroidBackListener>();
    const deps = dispatchDeps();
    const subscribe = vi.fn(() => installed.promise);
    const unregister = vi.fn(async () => {});
    const uninstall = installAndroidBackHandler({
      ...deps,
      platform: async () => "android",
      subscribe,
    });
    await vi.waitFor(() => expect(subscribe).toHaveBeenCalledOnce());
    uninstall();
    installed.resolve({ unregister });
    await vi.waitFor(() => expect(unregister).toHaveBeenCalledOnce());
  });

  it("keeps the generated Activity free of Wry ownership and uses one official AppPlugin subscription", () => {
    const activity = readFileSync(
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt",
      "utf8",
    );
    const app = readFileSync("src/App.tsx", "utf8");

    expect(activity).not.toContain("handleBackNavigation");
    expect(activity).not.toContain("OnBackPressedDispatcher");
    expect(app).toContain('import("@tauri-apps/api/app")');
    expect(app.match(/onBackButtonPress\(handler\)/g)).toHaveLength(1);
    expect(app).not.toContain('addEventListener("popstate"');
    expect(app).toContain("requestAndroidRootClose(\n    safeClose,");
    expect(app).toContain('safeClose.prepare()) !== "accepted"');
    expect(app).toMatch(/catch \{\s*\/\/ The native close attempt failed[\s\S]*?allowClose = false;[\s\S]*?safeClose\.reset\(\);[\s\S]*?closeInProgress = false;/);
  });
});
