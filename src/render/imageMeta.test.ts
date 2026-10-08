import { beforeAll, describe, expect, it } from "vitest";
import { parseImageMetaBrace } from "./imageMeta";
import { initParser } from "./parse";

beforeAll(initParser);

describe("UI-OG-C5-P6-IMAGEMETA: image metadata is read by the EDN reader, not a substring scan", () => {
  it("does not read a key-looking substring inside a quoted value", () => {
    expect(parseImageMetaBrace('{:title ":width 999", :width 20}')).toEqual({ width: "20px" });
    expect(parseImageMetaBrace('{:alt ":height 5", :height "40%"}')).toEqual({ height: "40%" });
  });
  it("keeps the OG shapes: bare number is px, quoted percent and px pass through", () => {
    expect(parseImageMetaBrace("{:width 200, :height 100}")).toEqual({ width: "200px", height: "100px" });
    expect(parseImageMetaBrace('{:width "40%"}')).toEqual({ width: "40%" });
    expect(parseImageMetaBrace('{:width "50px" :height 7px}')).toEqual({ width: "50px", height: "7px" });
    expect(parseImageMetaBrace("{:width 40%}")).toEqual({ width: "40%" });
  });
  it("ignores non-length values and unreadable braces", () => {
    expect(parseImageMetaBrace('{:width "wide"}')).toEqual({});
    expect(parseImageMetaBrace("{:width [1 2]}")).toEqual({});
    expect(parseImageMetaBrace("{:width 20")).toEqual({});
    expect(parseImageMetaBrace(undefined)).toEqual({});
  });
  it("uses the first of a repeated key", () => {
    expect(parseImageMetaBrace("{:width 1 :width 2}")).toEqual({ width: "1px" });
  });
});
