import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { SheetAggregateFooterCell } from "../components/SheetAggregateFooter";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.replaceChildren(); });

it("D18: the visible date footer skips impossible dates", () => {
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(() => <SheetAggregateFooterCell ownerId="dates" columnKey="prop:day"
    fn="earliest" values={["2026-02-31", "<2026-03-01 Sun>"]} />, host);
  expect(host.querySelector("button")?.textContent).toBe("2026-03-01 (1 skipped)");
});
