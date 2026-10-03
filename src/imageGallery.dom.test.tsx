import { afterEach, describe, expect, it } from "vitest";
import { galleryFor } from "./imageGallery";

afterEach(() => { document.body.innerHTML = ""; });

function add(src: string, x: number, y: number, cls = "inline-image") {
  const img = document.createElement("img");
  img.className = cls;
  img.setAttribute("src", src);
  img.getBoundingClientRect = () => ({ x, y, left: x, top: y, right: x, bottom: y, width: 0, height: 0, toJSON() {} }) as DOMRect;
  document.body.appendChild(img);
  return img;
}

describe("galleryFor (GH #501: OG open-lightbox over the rendered images)", () => {
  it("is reading order (y then x) starting at the clicked image, earlier ones last", () => {
    add("lower", 0, 500);
    const second = add("second", 100, 100);
    add("first", 0, 100);
    add("top", 0, 0);
    expect(galleryFor(second)).toEqual(["second", "lower", "top", "first"]);
  });
  it("ignores images that are not inline page images and empty srcs", () => {
    const a = add("a", 0, 0);
    add("avatar", 0, 10, "avatar");
    add("", 0, 20);
    expect(galleryFor(a)).toEqual(["a"]);
  });
});
