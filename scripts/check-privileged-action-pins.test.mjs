import assert from "node:assert/strict";
import test from "node:test";
import { privilegedActionPinProblems as check } from "./check-privileged-action-pins.mjs";

test("credential detection covers new workflows and each publishing authority", () => {
  for (const credential of [
    "permissions:\n  id-token: write", "permissions: {id-token: write}",
    "env:\n  KEY: ${{ secrets.NEW_PUBLISH_KEY }}", "secrets: inherit",
    "permissions:\n  contents: write", "permissions:\n  packages: write",
    "permissions: write-all", "permissions: {'contents': 'write'}",
    "env:\n  KEY: ${{ secrets['NEW_KEY'] }}",
  ]) {
    assert.equal(check(`${credential}\nsteps:\n  - uses: actions/checkout@v4`).length, 1, credential);
  }
  assert.deepEqual(check("# id-token: write\npermissions:\n  contents: read\nenv:\n  GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\nsteps:\n  - uses: actions/checkout@v4"), []);
});

test("all external actions need a full SHA and comment; local actions are exempt", () => {
  const prefix = "permissions:\n  id-token: write\nsteps:\n";
  assert.deepEqual(check(`${prefix}  - uses: ./local\n  - uses: owner/action@${"a".repeat(40)} # v1.2.3`), []);
  for (const ref of ["v4", "main", "a".repeat(39), "a".repeat(41)]) {
    assert.equal(check(`${prefix}  - uses: 'owner/action@${ref}' # v1`).length, 1);
  }
  assert.equal(check(`${prefix}  - uses: owner/action@${"a".repeat(40)}`).length, 1);
  assert.equal(check(`${prefix}  - { uses: actions/checkout@v4 }`).length, 1);
  assert.equal(check(`${prefix}  - 'uses': 'actions/checkout@v4'`).length, 1);
});
