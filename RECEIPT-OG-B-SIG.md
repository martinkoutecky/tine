# OG-B-SIG receipt

Base: `46f808ba6e918dc13170777541702339197e1daa`; branch `og-b-sig`.
Implementation commit: `db7645cf5` (parser/layout API preparation).

**Disposition: partial preparation, not task closure or delivery.** The native save signal, current-page static query family, and native parse-ratchet reduction remain pending the write-set gaps below. No finding is marked closed in the private parity ledger. No push, merge, integration, release build, deployment, dependency installation, real app-data access, or private brain access occurred.

Invariants in play: I-25, I-12, I-13/I-15, campaign E cost invariants.
Unit cost: no persisted record, format, file, edit-kind or transport change. Save IPC measurements are unchanged; see below.

## Step 0: scope and contract

This is a checkpoint-4 defect-repair lane, not a master feature port; master delta/inventory accounting is not applicable. The read-only master head is `5dfc8450310af0912755f963aa6bc4eaf4116c40`. The authorities are existing tine-store snapshots/publication and tine-core's lsdoc-owned outline. X-class machinery is neither needed nor introduced.

At base, `blockRefCounts.ts` watches `dataRev` and refetches the complete count map. `pageIndex.ts` also watches `dataRev` and rebuilds the inventory after ordinary saves. The native save wire returns revisions only. Static `RenderGraph::query_parsed` uses default ExecutionContext; `render_query_cache.rs` keys distinguish dialect/source only. `sheet_inputs` and `emit_query` both route through context-free query_rows. Baked publication uses a page context and privately substitutes current-page templates. Native `prepare_page_content` parses old content and then independently calls formatting detection.

The smallest shared invariant is that a derived answer changes only when its source contribution changes; query results also belong to their owning page. Native publication must produce the bounded count-target updates and name-inventory signal, shared by save and external changes. The frontend must consume them without deriving native metadata itself. Serialization should obtain old Document and layout from one parse. No unrelated migration or new write primitive is proposed.

Trust boundary: honest external-editor/sync races, malformed imported content, crash/power loss, and disk errors. The existing guarded transaction/temp/fsync/rename path remains authoritative. Guide exception: the committed work is an internal parser/layout API and bug correction, with no new capability, control, or workflow.

## Findings and evidence

- **L10:59 / L15:80: reproduced, pending.** A changed economy assertion in the existing page-index test fails on exact base: after bumpDataRev, expected one inventory call but got two. The real `setRaw -> flushPage` probe measures both refresh IPCs at 2k/10k pages. Its temporary test was removed after measurement; the original economy assertions are unchanged because a native signal consumer cannot safely replace them alone. The failing contract patch is preserved below, not installed as a failing gate.
- **L04:38: reproduced, pending.** The supplied current-page publication fixture fails through `publish_live`: the static owning-page query has no result list, while baked queries carry page context. The adapted regression also checks different expected referrers on Public and Second, preventing identical-source cache cross-contamination. It remains a receipt fixture until the producer/call sites are authorized; no passing proxy test is substituted for closure.
- **L05:30 remainder: preparation implemented, native wiring pending.** `doc::parse_with_opts` now returns Document plus SerializeOpts from the same parser-owned headers. Ordinary `doc::parse` shares the document builder without calculating unnecessary layout. SerializeOpts::detect uses the same layout answerer for standalone callers. No document schema or serde changes. The native caller still invokes standalone detect, so no save parse reduction is claimed and the ratchet remains 6.
- **Neighbor discovered during preparation: fixed.** A literal code bullet formerly supplied a bogus six-space indent. `formatting_detection_uses_only_parser_owned_headers` fails on base (six spaces vs tab) and passes after. Indentation now reads only lsdoc-owned structural prefixes; no new content grammar, regex, prefix classification, or literal scanner was introduced. Catalog: REG-OG-SIG-LAYOUT-001.

Class sites: doc::parse and SerializeOpts::detect share SerializeOpts::from_outline; parse_with_opts exposes their combined result. Native consumers at model.rs:detect_serialize_opts, cached-normalization detection, and prepare_page_content require integration. Current-page callers are RenderGraph::query_parsed/query_bounded, static render_query_with_title and its cache keys, render_sheets::query_rows/sheet_inputs/emit_query, and publish_query::baked_queries/substitute_current_page. Save consumers are createPage, single-page saves and grouped saves, blockRefCounts/pageIndex, and external single/bulk publications.

Prevention: fix parser-owned layout recognition; guard with a fail-before literal-bullet test and LF/CRLF/lone-CR structural roundtrips; shape exposes document + layout as one return value; prompt documents one outline parse and cost at the new API. Full signal/context family guards remain pending, not claimed.

## Save IPC measurements

Measured through the actual frontend save entry point with the same compact JSON inventory/count fixtures as the supplied B-W3 probe. Response bytes exclude the normal save result/RPC. Frontend and native save signal behavior is unchanged by the preparatory core commit.

| Pages | Phase | Inventory IPC / bytes | Count-map IPC / bytes | Extra IPC / bytes per save |
|---:|---|---:|---:|---:|
| 2,000 | before | 1 / 244,693 | 1 / 82,001 | 2 / 326,694 |
| 2,000 | after preparation | 1 / 244,693 | 1 / 82,001 | 2 / 326,694 |
| 10,000 | before | 1 / 1,236,693 | 1 / 410,001 | 2 / 1,646,694 |
| 10,000 | after preparation | 1 / 1,236,693 | 1 / 410,001 | 2 / 1,646,694 |

The requested zero-extra-IPC result has NOT been achieved. No native count/result-byte improvement or G5 closure is claimed.

## Write-set gaps / pending scope authorization

The batch common instruction is explicit: "Anything outside your write set: record as pending in the receipt and move on."

1. `crates/tine-store/src/model.rs` owns detect_serialize_opts and prepare_page_content; it is outside the task's core doc.rs/outline.rs API set. Wire the new combined parse there, retire the redundant detector call, accurately adjust its test counter, and lower the i13_edit_cost ratchet only after a real save proves the reduced count.
2. `crates/tine-graph-features/src/render.rs` owns RenderGraph::query_parsed/query_bounded, the static query cache construction, and Ctx (which has no owning page). It is absent from the write set. `publish_query.rs` owns baked_queries and the existing current-page substitution; no baked_queries* file exists at base. Share the substitution and page context at the query runner and include context in cache identity; sheet input/render calls must use that same runner.
3. The external notification producer (`src-tauri/src/watcher.rs`) and its consumers (`src/document/external.ts`, external single/bulk changes) are outside the write set. Inventory updates after externally changed aliases/names and count updates after external refs must remain correct when dataRev-based full refetching is removed. Do not install only the frontend suppression and silently lose those updates.

A concise asynchronous scope request was sent during implementation for the narrowly scoped moved sites. No answer arrived before the receipt. No permission was inferred from elapsed time. B-FAIL2 discovery/error paths, graph.rs and non-save commands, and B-DOOR2 query/refs/block-region functions were not edited.

## Gates

| Gate | Exit | Result |
|---|---:|---|
| Inventory new-contract necessity test on base | 1 | Expected one inventory call; got two after content revision. Preserved patch below. |
| Owning-page query necessity test on base | 101 | Static current-page query has no result rows through publish_live. Preserved patch below. |
| Parser-owned literal-layout test against base core | 101 | Six-space indent from a code example instead of tab. |
| Parser/layout tests on candidate | 0 | Both tests pass, including LF/CRLF/lone-CR roundtrips. |
| `timeout 180 npx tsc --noEmit` | 0 | Initial and final typechecks pass. |
| `timeout 1500 npm test -- --maxWorkers=4`, initial and after final formatting | 0 each | Node: 303 files / 2,381 tests pass; render: 264 files / 2,003 tests pass. Existing skips unchanged. No Errors line. |
| Root pinned-toolchain `cargo fmt --all -- --check` | 0 | One sanctioned root formatting operation; final check clean. |
| `CARGO_INCREMENTAL=0 timeout 3000 cargo test --workspace --tests --no-fail-fast`, initial | 101 | Only asset_watch target fails: no External Modified event for assets/pic.png, saw []. All other targets pass. |
| Exact failed asset-watch test rerun unchanged | 0 | Passes. No watcher/test change made; root cause of missed first event is unproven. |
| Full workspace command rerun on final production source | 0 | Every target passes, including asset_watch, native saves, layout, watch, query, and i13 ratchets. |
| `i13_edit_cost::edit_cost_is_page_bounded --exact --nocapture` | 0 | Still 6 parses / 1 old-source parse, on 1- and 60-block pages at 20/2,000 graph pages. Writes 8/539 bytes, one page file, four full reads and two fsyncs. Ratchet untouched. |
| `node scripts/check-ui-regression-catalog.mjs` | 0 | 308 entries / 157 GitHub issues; final textual append checked. |
| G6 untouched/roundtrip, debug adapter | 0 | 1,075 pages read; 0 unreadable; 0 opening byte diffs; 0 structural bugs; 999 acceptable canonicalizations, 76 byte-identical. |
| G6b one-block save, debug adapter | 0 | Frontend projection test passes; 1,004 saved pages pass; 71 no-leaf pages; 0 editable-page failures. |
| `git diff --check` | 0 | No whitespace defects. |

The initial workspace failure and unchanged reruns are retained in the logs; final green does not erase that observation. Manager-owned independent G2/G3 reviews, native G7 build/burn-in/deployment, final native IPC proof and complete task closure are not claimed.

## Size and final scope

Against the requested base: doc.rs +42/-26 = +16 production lines; new serialization_layout tests +34 lines (2.125x net production addition); CHANGELOG +4; catalog one textual appended row and its separator. No dependency, surface ratchet, format ratchet, allow-list or i13 ratchet changed. The receipt is committed at the user's explicit instruction, overriding the common untracked-receipt convention. Final production source remains db7645cf5; the final commit adds this receipt and normalizes only the placement of the additive changelog/catalog records. Final free space: 190 GB, above the 150 GB minimum.

Evidence logs are in `/tmp/og-b-sig-evidence/`. G6/G6b run unchanged scripts through a private cargo adapter removing only their hard-coded --release flag, with CARGO_INCREMENTAL=0 and this worktree's debug target. Their source corpus is copied; no private graph names/content or TSV output is tracked. A quoting error in the initial private adapter caused exit 1 before cargo; it was corrected and both gates rerun.

## Reproduction patches

These new failing checks were run and then removed from test discovery because their required fixes are outside the authorized set. Existing tests/assertions were restored unchanged; the fixture evidence is retained here.

### Inventory economy fail-before

```diff
diff --git a/src/pageIndex.test.ts b/src/pageIndex.test.ts
index 24790cdd4..ed5b389da 100644
--- a/src/pageIndex.test.ts
+++ b/src/pageIndex.test.ts
@@ -35,8 +35,8 @@ beforeEach(() => {
 describe("page index: the one frontend name answerer", () => {
   // Ported from graph.test "loads real page identities once and lets them win
   // colliding aliases": the backend's target is returned verbatim, and each
-  // trigger costs one page_inventory IPC (bind, content save, create/delete).
-  it("answers from the backend target and refetches once per trigger", async () => {
+  // inventory trigger costs one IPC; content-only saves cost none (I-25).
+  it("answers from the backend target and refetches only inventory changes", async () => {
     backendMock.pageInventory.mockResolvedValue(inventory(1,
       file("page1"),
       alias("shortcut", "pages/other.md"),
@@ -51,15 +51,16 @@ describe("page index: the one frontend name answerer", () => {
     expect(navigationName("Unknown")).toBe("Unknown");
 
     bumpDataRev();
-    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(2));
+    await new Promise((resolve) => setTimeout(resolve, 0));
+    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);
     bumpPageInventoryRev();
-    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(3));
+    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(2));
     // A save that bumps both in one tick costs one IPC, not two.
     bumpDataRev();
     bumpPageInventoryRev();
-    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(4));
+    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(3));
     await new Promise((resolve) => setTimeout(resolve, 0));
-    expect(backendMock.pageInventory).toHaveBeenCalledTimes(4);
+    expect(backendMock.pageInventory).toHaveBeenCalledTimes(3);
   });
 
   // Ported from graph.test "refreshes real-page precedence after a same-session
```

### Owning-page query fail-before

```diff
diff --git a/crates/tine-graph-features/tests/query_publication.rs b/crates/tine-graph-features/tests/query_publication.rs
index ad09eccc9..dabf4a1fc 100644
--- a/crates/tine-graph-features/tests/query_publication.rs
+++ b/crates/tine-graph-features/tests/query_publication.rs
@@ -34,6 +34,24 @@ fn bundle() -> Vec<(String, Vec<u8>)> {
          ("assets/app.js".into(), b"console.log('app')".to_vec())]
 }
 
+#[test]
+fn current_page_queries_match_baked_publication_context() {
+    let (graph, output, store) = fixture();
+    let query = "{:query [:find (pull ?b [*]) :in $ ?current-page :where [?p :block/name ?current-page] [?b :block/refs ?p]] :inputs [:current-page]}";
+    fs::write(graph.join("pages/Public.md"), format!("public:: true\n- {{{{query {query}}}}}\n")).unwrap();
+    fs::write(graph.join("pages/Second.md"), format!("public:: true\n- {{{{query {query}}}}}\n- from second [[Public]]\n")).unwrap();
+    fs::write(graph.join("pages/Third.md"), "public:: true\n- from third [[Second]]\n").unwrap();
+    store.scan_refresh().unwrap();
+    publish_live(&store, &output, "Context", false, &bundle()).unwrap();
+    for (owner, wanted, unwanted) in [("public", "from second", "from third"), ("second", "from third", "from second") ] {
+        let html = fs::read_to_string(output.join(format!("context/{owner}.html"))).unwrap();
+        let result = html.split("class=\"query-results\"").nth(1).expect("owning-page query must return rows").split("</ul>").next().unwrap();
+        assert!(result.contains(wanted) && !result.contains(unwanted), "I-12: static queries and their cache must use the owning page; exemplar render_query_cache.rs: {result}");
+    }
+    store.close();
+    fs::remove_dir_all(graph.parent().unwrap()).unwrap();
+}
+
 #[test]
 fn query_publication_reviews_owner_pages_and_rejects_a_stale_plan() {
     let (graph, output, store) = fixture();
```

### Save IPC probe

```typescript
import { expect, it, vi } from "vitest";
vi.mock("./warmCache", () => ({ waitForWarmCache: async () => true }));
import { backend } from "./backend";
import { blockRefCount } from "./blockRefCounts";
import { installPageIndex } from "./pageIndex";
import { loadFeed, setRaw, resetStore, flushPage } from "./document";
import { bumpGraphEpoch } from "./graphSession";
it.each([2000,10000])("ordinary save IPC on %i pages", async (pages) => {
 resetStore();
 const entries = Array.from({length:pages}, (_,i)=> ({ key: `p${i}`, name: `P${i}`, is_journal:false, day:null, target:{ kind:"existing" as const, id:`pages/P${i}.md`, others:[] } }));
 const counts = Object.fromEntries(entries.map((_,i)=>[`00000000-0000-4000-8000-${String(i).padStart(12,"0")}`,1]));
 const inventory = {rev:"1",entries};
 const names = vi.spyOn(backend(),"pageInventory").mockResolvedValue(inventory);
 const refs = vi.spyOn(backend(),"getBlockRefCounts").mockResolvedValue(counts);
 vi.spyOn(backend(),"savePages").mockResolvedValue({ok:["saved"]});
 installPageIndex(); bumpGraphEpoch(); blockRefCount("body");
 await vi.waitFor(()=>expect(names.mock.calls.length).toBeGreaterThan(0));
 await vi.waitFor(()=>expect(refs.mock.calls.length).toBeGreaterThan(0));
 names.mockClear(); refs.mockClear();
 loadFeed([{id:"pages/P0.md",rev:"old",name:"P0",kind:"page",title:"P0",pre_block:null,blocks:[{id:"body",raw:"before",collapsed:false,children:[]}]}]);
 setRaw("body","after"); expect(await flushPage("P0")).toBe(true);
 await vi.waitFor(()=>expect(names).toHaveBeenCalledTimes(1));
 await vi.waitFor(()=>expect(refs).toHaveBeenCalledTimes(1));
 console.log(JSON.stringify({pages,inventoryCalls:names.mock.calls.length,inventoryResponseBytes:Buffer.byteLength(JSON.stringify(inventory)),countCalls:refs.mock.calls.length,countResponseBytes:Buffer.byteLength(JSON.stringify(counts))}));
 resetStore(); vi.restoreAllMocks();
});
```
