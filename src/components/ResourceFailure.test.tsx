import { afterEach, describe, expect, it, vi } from "vitest";
import type { Resource } from "solid-js";
import { render } from "solid-js/web";
import { ResourceFailure } from "./ResourceFailure";

afterEach(() => {
  document.body.innerHTML = "";
});

/** The two states a panel cares about, without waiting on a real fetch. */
function resourceStub(error?: unknown): Resource<unknown> {
  const read = (() => undefined) as unknown as Resource<unknown>;
  Object.defineProperty(read, "error", { get: () => error });
  return read;
}

function mount(element: () => ReturnType<typeof ResourceFailure>) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  render(element, host);
  return host;
}

describe("ResourceFailure: an empty panel must not claim there is nothing", () => {
  it("says nothing while the resource is fine", () => {
    const host = mount(() => <ResourceFailure of={resourceStub()} what="references to this block" />);
    expect(host.querySelector(".resource-failure")).toBeNull();
  });

  it("names what could not be loaded, as an alert", () => {
    const host = mount(() => (
      <ResourceFailure of={resourceStub(new Error("boom"))} what="references to this block" />
    ));
    const row = host.querySelector<HTMLElement>(".resource-failure")!;
    expect(row.getAttribute("role")).toBe("alert");
    expect(row.textContent).toContain("Couldn’t load references to this block.");
  });

  it("does not show the failure's message, which can carry the user's own content", () => {
    const host = mount(() => (
      <ResourceFailure of={resourceStub(new Error("no such page /home/me/graph/Secret.md"))} what="references" />
    ));
    expect(host.querySelector(".resource-failure")!.textContent).not.toContain("Secret");
  });

  it("offers Retry only when the panel has one, and calls it", () => {
    const withoutRetry = mount(() => <ResourceFailure of={resourceStub(new Error("boom"))} what="references" />);
    expect(withoutRetry.querySelector(".resource-failure-retry")).toBeNull();

    const onRetry = vi.fn();
    const withRetry = mount(() => (
      <ResourceFailure of={resourceStub(new Error("boom"))} what="references" onRetry={onRetry} />
    ));
    withRetry.querySelector<HTMLButtonElement>(".resource-failure-retry")!.click();
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("shows when ANY of several resources failed", () => {
    const host = mount(() => (
      <ResourceFailure of={[resourceStub(), resourceStub(new Error("boom"))]} what="this query" />
    ));
    expect(host.querySelector(".resource-failure")).not.toBeNull();
  });
});
