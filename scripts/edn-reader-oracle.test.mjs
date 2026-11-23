// nbb 1.2.187 bundles ClojureScript 1.11.60 and tools.reader 1.3.6.
// Its two CLJS reader sources are byte-identical to Logseq's 1.11.54 copies.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

const first = "observe-first";
const strict = "observe-strict";
const stream = "observe-stream";
const units = String.raw;

const literals = [
  "nil", "true", "false", "one", "one/two", ":a", ":a/b", ":", "::a", ":a::b",
  ":a/", ":a//", ":a/b/c", "a/b/c", ":a/1", ":1", ":/a", ":a:/b", "/",
  "0", "-0", "-00", "-0x0", "1N", "1.0M", "01M", "01.5", "01e2", "2.e-1", "1e309",
  "0x12", "0X12", "-0x12", "0x12N", "022", "08", "2r10", "2R10z",
  "16r0x12", "36rN", "2r2", "37r1", "1.1N", "+1/2", "1/0", "0/0", "1/-2",
  "9007199254740993", "0x10000000000000801", "##NaN", "##Inf", "##-Inf",
  units`\o1`, units`\o12`, units`\o101`, units`\o400`, units`\o+01`, units`\u+041`,
  units`\u0041`, units`\uD800`, units`\newline`, units`\;`, units`\\" `.trim(), units`\😀`,
  '"plain"', '"😀"', units`"\t\r\n\\\"\b\f"`, units`"\uD800"`, units`"\uDC00"`,
  units`"\uD83D\uDE00"`, units`"\uD800a"`, units`"\uD800\u0041"`, units`"\u+041"`,
  units`"\1"`, units`"\12"`, units`"\101"`, units`"\400"`, units`"\8"`, units`"\é"`,
  '#inst "2020-02-29T00:30+01:00"', '#inst "2020-02-28T23:30Z"', '#inst "1970"',
  '#inst "2025-02-29"', '#inst "2024-01-02T12:34:56.24Z"', '#inst "2024-01-02T12:34:56."',
  '#inst "2024-01-02T12:34:56.245678Z"', '#uuid "ABC"', units`#uuid "\uD800"`,
  "#queue [1]", "#js [1]", '#js {:a/x 1 :b/x 2}', "#js {foo 1}", "#js {1 2}",
  units`#js {\a "\uDC00"}`, units`#js {"\uD800" "\uDC00"}`, units`#js ["\uD800"]`,
  '#js {"__proto__" 42 "constructor" 1}', '#unknown []', '#(inc %)', '#"x"', "#'a",
  '#js {"/" 1 :/ 2 / 3}', '#js {"__proto__" nil :ns/__proto__ 1}',
  '#js {:a/x 1 :b/x 2 :c 0 :d 0 :e 0 :f 0 :g 0 :h 0 :i 0}',
  "#:ns{:a 1 :_/b 2 c 3 :other/d 4}", "#:ns {:a 1}", "#:ns{:a 1 :ns/a 2}",
  units`#:ns{:_/title "\uD800" :a "\uDC00"}`, "#:ns/name{:a 1}", "#::ns{:a 1}",
  '^:tag [1]', '#^"tag" [1]', '^:a #^:b [:a]', '^{:p [:private]} {:public []}',
  '^:tag 1', '^:private #queue [1]', '{[1] :a (1.0) :b}', '#{#queue [1] [1]}',
  '#{#{1 2} #{2 1}}', '{{:x 1 :y 2} :a {:y 2 :x 1} :b}', '#{0x12 18}',
  units`{\a 1 "a" 2}`, units`#{"😀" "\uD83D\uDE00"}`,
  '#{1N 1.0M}', '#{##NaN ##NaN}', '#{0/0 ##NaN}', '#{#uuid "ABC" #uuid "abc"}',
  '#{#inst "2020-02-29T00:30+01:00" #inst "2020-02-28T23:30Z"}',
  '#{#js [] #js []}', '#{#js {:a/x 1 :b/x 2} #js {:x 2}}',
  '#{#_ {:x 1 :x 2} :public}', units`#{\u+041 :Private}`, units`#{[\u+041] :Private}`,
  units`#{#_ \u+041 :Private}`, "#_:first 17", "#_ #_ :a :b :c",
];

const discardedTags = ['#inst "2020"', '#uuid "ABC"', '#queue []', '#js []', '#js {:a 1}'];
const discardedForms = [
  "nil", "true", "false", "symbol", ":hidden", "1", "##Inf", units`\newline`, '":hidden"',
  '[:hidden]', '(:hidden)', '#{:hidden}', '{:hidden []}', '#:demo{:_/hidden []}',
  '^:meta {:hidden []}', '#^:meta {:hidden []}', '#_ :dropped :hidden',
  ...discardedTags,
];
const onlyTrivia = [
  " ,", "; {:hidden []}", "#! {:hidden []}",
  ...discardedForms.map((form) => `#_ ${form}`),
].flatMap((source) => ["", "\n", "\r", "\r\n"].map((end) => source + end));
const witnesses = [
  "", " , \n", "#_ :dropped", "; just a comment", "1 #unknown []", "{:pages-directory \"notes\"} #unknown []",
  "{:pages-directory \"notes\" :bad #unknown []}",
  // `##` with a complete operand other than NaN/Inf/-Inf: upstream rejects it.
  ...["##unknown", "##nil", "##true", "##1", "##:kw", '##"s"', units`##\c`, "##[]", "##()", "##{}", "###{}"]
    .flatMap((value) => [value, `{:bad ${value} :hidden []}`]),
  ...literals,
  ...onlyTrivia,
  ...discardedTags.flatMap((tag) => [`[#_ ${tag} 1]`, `[#_ [${tag}] 1]`, `#_ ${tag} 1`]),
  ...[";", "#!"].flatMap((comment) =>
    ["\n", "\r", "\r\n"].flatMap((end) => [
      `[${comment} ignore${end} :kept\n]`, `{:query [:find ?b :where [?b :block/marker "TODO"]] ${comment} ignore${end} :note 1\n}`,
    ])),
];

// EDN model R4b. Forms upstream reads to their end: the stream observation checks where
// each one ends, so local config recovery resumes where upstream would.
const extentControls = [
  "#_ #^:m [] :after", "#_ #_ 1 2 :after", '#uuid #_ 1 "x" :after', '# inst "2020-01-01" :after',
  '#; comment\ninst "2020-01-01" :after', '# ^:m inst "2020-01-01" :after',
  '# #_ 0 inst "2020-01-01" :after', "#_ #:demo ,{:_/hidden []} :after", "#_ (1#{} 2) :after",
  units`#_ (a^:m [] \; 1:kw) :after`, "##Inf :after", "## Inf :after", "#{1} [] :after",
  "x#! :hidden", "1#! :hidden\n:after", units`\;:hidden`,
  "#_ #_ 1", units`\c:hidden`, "#:demo{:_/hidden [] :x 1} :after",
];
// Values local config reading refuses (R4b, both model reviews): the strict reader
// refuses each one where upstream does, directly and as a map value.
const refusedValues = [
  "##unknown", "##false", "#1 []", "#nil []", "#false []", '#"s" []', units`#\x []`, "#[1] []",
  "#(x) []", "#=x []", "#'x []", "#? (:cljs [])", "## #1 []", "## #(x) []", "## #? (:cljs [])",
  units`\malformed`, '#_ #inst "2020-01-01" 1', "## #{1} []", "#()", '#"r"', "#'x", "## #()",
  "#() [] 1", "#<x>", "#<", "#?@ (:cljs [])", "## #<x>", "#:kw []", "#:demo []",
  "#:demo #_ [] {:x 1}", "#:demo ;comment\n{:x 1}", "#:demo", "#:bad/ns{}", "#:/bad {}",
  "#:bad: {}", "#:{}", "## #:demo []", "[) ]", units`"\u"`, units`"bad\q"`, units`"\u12"`,
  "[#()]", "[#() []]", "{:x #()}", "^:m #()", "^:m #() []", "#^:m #() []", "#x #() []",
  "#_ #()", "#_ #() []", "## ## #() []", "#?@:hidden", "a@b", ":a@b", ": x", units`\a@`, "^1 []",
  "{:a 1 :a 2}", "#:demo{:_/hidden [] :_/hidden []}",
];
witnesses.push(...extentControls, ...refusedValues,
  ...refusedValues.map((value) => `{:bad ${value} :hidden []}`));

// A fixed seed exercises dispatch, metadata, and discards at varying depths.
let seed = 0x56d2ed;
function choose(items) {
  seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
  return items[(seed >>> 12) % items.length];
}

function generate(depth) {
  if (depth === 0) return choose(literals);
  const value = generate(depth - 1);
  return choose([
    `[${value} :end]`, `(${value} :end)`, `#{${value} :end}`,
    `{:value ${value}}`, `#:ns{:_/value ${value}}`, `#js [${value}]`,
    `#queue [${value}]`, `^:meta [${value}]`, `[#_ ${value} :end]`,
    `[#! hidden${choose(["\n", "\r", "\r\n"])} ${value}\n]`,
  ]);
}

for (let i = 0; i < 256; i++) witnesses.push(generate(i % 5 + 1));

const radixWitness = "9r0317076505470440520";
const radixDuplicates = [
  `{${radixWitness} :a 53345449898862296 :b}`,
  `#{${radixWitness} 53345449898862296}`,
];
const radixLiterals = [radixWitness, "-9r000", "+10R000", "2r000"];
for (let radix = 2; radix <= 36; radix++) {
  const digits = "0123456789abcdefghijklmnopqrstuvwxyz".slice(0, radix);
  const chunk = Math.floor(Math.log(0xffffffff / 36) / Math.log(radix));
  const safeInteger = Number.MAX_SAFE_INTEGER.toString(radix).length;
  // Straddle each boundary in both the significant digits and the full spelling.
  const lengths = new Set([chunk, 2 * chunk, safeInteger]
    .flatMap((boundary) => [boundary - 2, boundary - 1, boundary, boundary + 1]));
  for (const length of lengths) {
    for (const sign of ["", "+", "-"]) {
      for (const separator of ["r", "R"]) {
        const value = choose(digits.slice(1)) +
          Array.from({ length: length - 1 }, () => choose(digits)).join("");
        radixLiterals.push(`${sign}${radix}${separator}0${value}`);
      }
    }
  }
}
witnesses.push(...radixLiterals, ...radixDuplicates);

const query = '[:find ?b :where (task ?b #{"TODO" #_ "DONE"})]';
const streams = [
  query,
  `${query} :inputs [#_ :ignored :current-page] :note #:ns{:_/key 1}`,
  ...discardedTags.flatMap((tag) => [`${query} :note ${tag}`, `${query} #_ ${tag} :note 1`]),
  ...literals.map((literal) => `${query} :note ${literal}`),
  ...radixDuplicates.map((literal) => `${query} :note [{:nested ${literal}}]`),
  ...extentControls,
];
const cases = [
  ...[...new Set(witnesses)].flatMap((source) => [[first, source], [strict, source]]),
  ...streams.map((source) => [stream, source]),
];

function canonical(observation) {
  if (observation === null) return null;
  const [kind, value] = observation;
  const sort = (items) => items.sort((a, b) => {
    const left = JSON.stringify(a);
    const right = JSON.stringify(b);
    return left < right ? -1 : Number(left > right);
  });
  switch (kind) {
    case "map": return [kind, sort(value.map(([k, v]) => [canonical(k), canonical(v)]))];
    case "js-map": return [kind, sort(value.map(([k, v]) => [k, canonical(v)]))];
    case "set": return [kind, sort(value.map(canonical))];
    case "stream": case "list": case "vector": case "queue": case "js-vector":
      return [kind, value.map(canonical)];
    default: return observation;
  }
}

test("Tine's Wasm EDN reader agrees with Logseq's cljs.reader", async () => {
  const dir = resolve(process.env.TINE_EDN_ORACLE_WASM || "target/edn-oracle-wasm");
  const wasm = await import(pathToFileURL(resolve(dir, "lsdoc_wasm.js")).href);
  await wasm.default({ module_or_path: readFileSync(resolve(dir, "lsdoc_wasm_bg.wasm")) });
  const expected = JSON.parse(execFileSync(process.execPath, [
    "node_modules/nbb/cli.js", "scripts/edn-reader-oracle.cljs",
  ], { input: JSON.stringify(cases), encoding: "utf8", maxBuffer: 4 * 1024 * 1024 }));
  assert.equal(expected.length, cases.length);
  const failures = [];
  for (const [i, [operation, source]] of cases.entries()) {
    const actual = JSON.parse(wasm.query_edn_json(source, operation, ""));
    const [wanted, got] = [expected[i], actual].map(canonical).map(JSON.stringify);
    if (wanted !== got) failures.push({ operation, source, wanted, got });
  }
  console.log(`EDN oracle: ${failures.length}/${cases.length} disagreements`);
  assert.equal(failures.length, 0, failures.map((failure) => JSON.stringify(failure)).join("\n"));
});
