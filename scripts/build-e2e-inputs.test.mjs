import assert from "node:assert/strict";
import test from "node:test";
import { allowsLocalDirtyBuild } from "./build-e2e-inputs.mjs";

test("uncommitted native candidates require explicit local ordinary-mode opt-in", () => {
  assert.equal(allowsLocalDirtyBuild("ordinary", {}), false);
  assert.equal(allowsLocalDirtyBuild("ordinary", { TINE_E2E_ALLOW_DIRTY_BUILD: "0" }), false);
  assert.equal(allowsLocalDirtyBuild("ordinary", { TINE_E2E_ALLOW_DIRTY_BUILD: "1" }), true);
  assert.equal(allowsLocalDirtyBuild("ordinary", { TINE_E2E_ALLOW_DIRTY_BUILD: "1", CI: "true" }), false);
  assert.equal(allowsLocalDirtyBuild("ordinary", { TINE_E2E_ALLOW_DIRTY_BUILD: "1", CI: "1" }), false);
  assert.equal(allowsLocalDirtyBuild("release", { TINE_E2E_ALLOW_DIRTY_BUILD: "1" }), false);
});
