import assert from "node:assert/strict";
import { normalizeAst } from "../src/devtools/lsdoc-diff/vendor/normalize.mjs";

const at = { start_pos: 0, end_pos: 1 };
assert.deepEqual(
  normalizeAst([
    [["Export", "html", "quoted", "<b>x</b>"], at],
    [["CommentBlock", ["one\n", "two\n"]], at],
  ]),
  [
    { kind: "export", name: "html", options: "quoted", content: "<b>x</b>", span: [0, 1] },
    { kind: "comment_block", content: "one\ntwo\n", span: [0, 1] },
  ],
  "the vendored normalizer must understand every canonical lsdoc block kind",
);

console.log("lsdoc oracle behavior OK");
