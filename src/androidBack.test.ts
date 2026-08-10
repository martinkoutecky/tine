import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import {
  dispatchAndroidBack,
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
} {
  const state = {
    transient: false,
    drawer: false,
    dismissTransient: vi.fn(() => state.transient),
    dismissDrawer: vi.fn(() => state.drawer),
    restoreDrawerFocus: vi.fn(),
    historyBack: vi.fn(),
    closeRoot: vi.fn(),
  };
  return state;
}

describe("GH #161 Android SafeBack owner", () => {
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

    expect(dispatchAndroidBack({ canGoBack: false }, deps)).toBe("root");
    expect(deps.historyBack).toHaveBeenCalledOnce();
    expect(deps.closeRoot).toHaveBeenCalledOnce();
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

  it("leaves the native SafeBack owner blocking when platform or subscription setup rejects", async () => {
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

  it("uses a permanent SafeBack owner and never revives the AppPlugin fallback", () => {
    const activity = readFileSync(
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt",
      "utf8",
    );
    const safeBackPlugin = readFileSync(
      "src-tauri/gen/android/app/src/main/java/page/tine/app/SafeBackPlugin.kt",
      "utf8",
    );
    const app = readFileSync("src/App.tsx", "utf8");

    expect(activity).not.toContain("handleBackNavigation");
    expect(activity).toContain("OnBackPressedCallback(true)");
    expect(activity).toContain("SafeBackBridge.dispatchIfReady()");
    expect(activity).toContain("showBlockedBackNotice()");
    expect(activity).toContain("SafeBackBridge.clear()");
    expect(activity).not.toContain("onBackPressedDispatcher.onBackPressed");
    expect(app).not.toContain('@tauri-apps/api/app');
    expect(app).toContain('addPluginListener("safe-back", "android-safe-back", handler)');
    expect(app).not.toContain("onBackButtonPress");
    expect(app).not.toContain('addEventListener("popstate"');
    expect(app).toContain("createAndroidRootCloseCoordinator(safeClose, {");
    expect(app).toContain('safeClose.prepare()) !== "accepted"');
    expect(app).toMatch(/catch \{\s*\/\/ The native close attempt failed[\s\S]*?allowClose = false;[\s\S]*?safeClose\.reset\(\);[\s\S]*?closeInProgress = false;/);
    expect(safeBackPlugin).toContain("private var webView: WebView? = null");
    expect(safeBackPlugin).toContain('hasListener("android-safe-back")');
    expect(safeBackPlugin).toContain('trigger("android-safe-back"');
    expect(safeBackPlugin).toContain("fun dispatchIfReady(): Boolean");
    expect(safeBackPlugin).not.toContain(".goBack()");
    expect(safeBackPlugin).not.toContain("activity.onBackPressed()");
    expect(safeBackPlugin).not.toContain("isEnabled = false");
  });

  it("keeps managed Android root Back as typed preparation then explicit activity exit", () => {
    const app = readFileSync("src/App.tsx", "utf8");
    const androidBack = readFileSync("src/androidBack.ts", "utf8");
    const backend = readFileSync("src/backend.ts", "utf8");
    const commands = readFileSync("src-tauri/src/commands.rs", "utf8");
    const lib = readFileSync("src-tauri/src/lib.rs", "utf8");
    const nativePlugin = readFileSync("src-tauri/src/android_safe_back.rs", "utf8");

    expect(backend).toContain("prepareQuit(): Promise<TineQuitPreparation>");
    expect(backend).toContain('return this.call<TineQuitPreparation>("prepare_tine_quit");');
    expect(app).toContain("prepareNativeClose: () => backend().prepareQuit()");
    expect(app).toContain('failure.status === "refused" || failure.status === "partial"');
    expect(app).toContain("Tine-managed storage could not verify a clean stop.");
    expect(app).toContain("Couldn't close the app. Your graph remains open.");
    expect(androidBack).toContain("NativePartiallyPrepared");
    expect(androidBack).toContain("native_prepare_uncertain");
    expect(androidBack).toContain("native_prepare_refused");
    expect(androidBack).toContain("native_prepare_partial");
    expect(app).toContain('await invoke("plugin:app|exit");');
    expect(commands).toContain("fn prepare_tine_quit_all_slots");
    expect(commands).toContain("enum TineQuitPreparation");
    expect(commands).toMatch(/Partial\s*\{\s*safe_slots: Vec<String>,\s*detail: String,\s*\}/);
    expect(commands).toContain("slots.sort_by");
    expect(commands).toContain("TineQuitPreparation::Safe");
    expect(commands).toMatch(/pub\(crate\) fn prepare_tine_quit\([\s\S]*?prepare_tine_quit_all_slots\(&state\)/);
    expect(commands).toMatch(/pub\(crate\) fn tine_quit\([\s\S]*?prepare_tine_quit_all_slots\(&state\)[\s\S]*?app\.exit\(0\)/);
    expect(lib).toMatch(/generate_handler!\[[\s\S]*?prepare_tine_quit,[\s\S]*?tine_quit,/);
    expect(lib).toContain("builder.plugin(android_safe_back::init())");
    expect(nativePlugin).toContain('Builder::new("safe-back")');
    expect(nativePlugin).toContain('register_android_plugin(PLUGIN_IDENTIFIER, "SafeBackPlugin")');
  });
});
