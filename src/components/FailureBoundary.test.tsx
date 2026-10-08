import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createResource, createSignal } from "solid-js";
import { render } from "solid-js/web";
import { FailureBoundary } from "./FailureBoundary";
import { markCommandSlow, resetSlowBackendStateForTests } from "../slowBackend";
import { setToasts, toasts } from "../toasts";

const { recordDiagnostic, copyDetails } = vi.hoisted(() => ({ recordDiagnostic: vi.fn(async () => {}), copyDetails: vi.fn(async (_text: string) => {}) }));
vi.mock("../clipboard", () => ({ writeClipboardText: copyDetails }));
vi.mock("../debug", () => ({ dbg: () => {}, recordDiagnostic }));

beforeEach(() => {
  setToasts([]);
  resetSlowBackendStateForTests();
  recordDiagnostic.mockClear();
  copyDetails.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function Boom(props: { when: () => boolean }) {
  if (props.when()) throw new Error("list_pages failed: backend is busy");
  return <div class="survivor">content</div>;
}

describe("FailureBoundary (GH #490/#332: a throw must not blank the app silently)", () => {
  it("shows the failing region, its message, and a Retry instead of rendering nothing", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    render(
      () => (
        <FailureBoundary region="This page">
          <Boom when={() => true} />
        </FailureBoundary>
      ),
      host,
    );
    const failure = host.querySelector<HTMLElement>(".region-failure")!;
    expect(failure).not.toBeNull();
    expect(failure.getAttribute("role")).toBe("alert");
    expect(failure.getAttribute("data-failed-region")).toBe("This page");
    expect(failure.textContent).toContain("This page could not be displayed.");
    expect(failure.textContent).toContain("list_pages failed: backend is busy");
    expect(host.querySelector(".region-failure-retry")).not.toBeNull();
  });

  it("keeps a sibling region alive when one region throws", () => {
    // The whole point: Solid drops every effect batched with the throw, so
    // before this boundary existed an unrelated panel went blank too.
    const host = document.createElement("div");
    document.body.appendChild(host);
    render(
      () => (
        <>
          <FailureBoundary region="This page">
            <Boom when={() => true} />
          </FailureBoundary>
          <FailureBoundary region="The sidebar">
            <div class="sidebar-survivor">still here</div>
          </FailureBoundary>
        </>
      ),
      host,
    );
    expect(host.querySelectorAll(".region-failure").length).toBe(1);
    expect(host.querySelector(".sidebar-survivor")?.textContent).toBe("still here");
  });

  it("Retry re-renders the region, so a transient failure recovers without a restart", () => {
    const [failing, setFailing] = createSignal(true);
    const host = document.createElement("div");
    document.body.appendChild(host);
    render(
      () => (
        <FailureBoundary region="This page">
          <Boom when={failing} />
        </FailureBoundary>
      ),
      host,
    );
    expect(host.querySelector(".region-failure")).not.toBeNull();

    setFailing(false);
    host.querySelector<HTMLButtonElement>(".region-failure-retry")!.click();

    expect(host.querySelector(".region-failure")).toBeNull();
    expect(host.querySelector(".survivor")?.textContent).toBe("content");
  });

  it("says so out loud, once, and records only a fixed kind in the always-on recorder", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    render(
      () => (
        <FailureBoundary region="Linked References">
          <Boom when={() => true} />
        </FailureBoundary>
      ),
      host,
    );
    expect(toasts().map((toast) => toast.kind)).toEqual(["error"]);
    expect(toasts()[0].message).toContain("Linked References");
    expect(recordDiagnostic).toHaveBeenCalledTimes(1);
    expect(recordDiagnostic).toHaveBeenCalledWith("uncaught_error");
  });

  it("names the backend's outstanding slow operations when it is the likely cause (GH #332)", () => {
    const settle = markCommandSlow(performance.now() - 37_000);
    const host = document.createElement("div");
    document.body.appendChild(host);
    render(
      () => (
        <FailureBoundary region="This page">
          <Boom when={() => true} />
        </FailureBoundary>
      ),
      host,
    );
    const slow = host.querySelector(".region-failure-slow")!;
    expect(slow.textContent).toContain("an operation");
    expect(slow.textContent).toMatch(/over 3[67]s/);
    expect(slow.textContent).toContain("notes on disk are not affected");
    settle();
  });

  it("stays quiet about the backend when nothing is slow", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    render(
      () => (
        <FailureBoundary region="This page">
          <Boom when={() => true} />
        </FailureBoundary>
      ),
      host,
    );
    expect(host.querySelector(".region-failure-slow")).toBeNull();
  });

  it("copies the region and original error details and reports clipboard failure", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <FailureBoundary region="This page"><Boom when={() => true} /></FailureBoundary>, host);
    const copy = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Copy details");
    expect(copy).toBeDefined();
    copy!.click();
    await Promise.resolve();
    expect(copyDetails).toHaveBeenCalledWith(expect.stringContaining("This page"));
    expect(copyDetails.mock.calls[0][0]).toContain("list_pages failed: backend is busy");
    copyDetails.mockRejectedValueOnce(new Error("clipboard denied"));
    copy!.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(toasts().some(toast => toast.kind === "error" && toast.sticky && toast.message.includes("copy"))).toBe(true);
    dispose();
  });

  it("contains a rejected resource read and Retry creates a fresh request", async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error("panel read failed")).mockResolvedValue("loaded after retry");
    function Panel() {
      const [data] = createResource(load);
      return <div class="loaded-panel">{data()}</div>;
    }
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <><FailureBoundary region="Panel"><Panel /></FailureBoundary><button class="healthy">Navigation</button></>, host);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(host.querySelector(".region-failure")?.textContent).toContain("panel read failed");
    expect(host.querySelector(".healthy")?.textContent).toBe("Navigation");
    host.querySelector<HTMLButtonElement>(".region-failure-retry")!.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(host.querySelector(".loaded-panel")?.textContent).toBe("loaded after retry");
    expect(load).toHaveBeenCalledTimes(2);
    dispose();
  });

  // Control arm, kept deliberately: this is what Tine shipped before the
  // boundary existed, and it is why the three tests above are not decoration.
  // Runs last because the escaping throw leaves Solid's globals mid-update.
  it("control: with no boundary the throw escapes render and the healthy sibling never mounts", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    expect(() =>
      render(
        () => (
          <>
            <Boom when={() => true} />
            <div class="sidebar-survivor">still here</div>
          </>
        ),
        host,
      ),
    ).toThrow("list_pages failed");
    expect(host.querySelector(".sidebar-survivor")).toBeNull();
    expect(host.querySelector(".region-failure")).toBeNull();
  });
});
