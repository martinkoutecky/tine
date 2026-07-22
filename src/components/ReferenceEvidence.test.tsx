import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { OccurrenceControls } from "./ReferenceEvidence";
import type { ReferenceBlockEvidence, ReferenceOccurrence } from "../types";

afterEach(() => {
  document.body.innerHTML = "";
});

const occ = (start: number): ReferenceOccurrence => ({
  matched_name: "Books",
  canonical: "books",
  kind: "explicit",
  span: { start, end: start + 5 },
  rule: "page-ref",
});
const evidence = (n: number): ReferenceBlockEvidence => ({
  block_id: "b1",
  occurrences: Array.from({ length: n }, (_, i) => occ(i * 10)),
});

describe("OccurrenceControls (GH #200: no redundant count/jump for a single mention)", () => {
  it("renders nothing when a block mentions the page once", () => {
    const host = document.createElement("div");
    render(() => <OccurrenceControls evidence={evidence(1)} onOccurrence={() => {}} />, host);
    expect(host.querySelector(".reference-occurrence-controls")).toBeNull();
    expect(host.textContent ?? "").not.toContain("mention");
  });

  it("shows the count and one jump button per occurrence when a block mentions the page 2+ times", () => {
    const host = document.createElement("div");
    render(() => <OccurrenceControls evidence={evidence(3)} onOccurrence={() => {}} />, host);
    expect(host.querySelector(".reference-occurrence-controls")).not.toBeNull();
    expect(host.textContent ?? "").toContain("3 mentions");
    expect(host.querySelectorAll(".reference-occurrence-jump").length).toBe(3);
  });
});
