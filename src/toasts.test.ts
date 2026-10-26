import { beforeEach, describe, expect, it } from "vitest";
import { pushReferenceChangeNotice, setToasts, toasts } from "./toasts";

beforeEach(() => setToasts([]));
describe("reference-change notices", () => {
  it("suppresses zero and keeps an affected-reference notice readable with Undo", () => {
    expect(pushReferenceChangeNotice(0, false, () => {})).toBeNull();
    let undos = 0;
    pushReferenceChangeNotice(3, false, () => { undos++; });
    expect(toasts()[0]).toMatchObject({ message: "3 references are now broken", kind: "warn", sticky: true });
    toasts()[0].action!.run();
    expect(undos).toBe(1); expect(toasts()).toEqual([]);
  });
  it("an unavailable count is described without inventing a number", () => {
    pushReferenceChangeNotice(null, true, () => {});
    expect(toasts()[0].message).toBe("References may now point to this block");
  });
});
