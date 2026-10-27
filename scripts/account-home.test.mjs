import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { once } from "node:events";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

test("native fixture requires a real held app lock and rejects fallback stderr", { skip: process.platform !== "linux" }, async () => {
  fs.mkdirSync("artifacts/e2e-owner-home", { recursive: true });
  const run = fs.mkdtempSync(path.resolve("artifacts/e2e-owner-home/helper-test-"));
  const fixture = accountHomeFixture(run);
  const lookup = () => JSON.parse(execFileSync("python3", ["-c", `
import ctypes, json, os
class Passwd(ctypes.Structure):
    _fields_ = [('name', ctypes.c_char_p), ('password', ctypes.c_char_p),
                ('uid', ctypes.c_uint), ('gid', ctypes.c_uint),
                ('gecos', ctypes.c_char_p), ('home', ctypes.c_char_p), ('shell', ctypes.c_char_p)]
entry = Passwd(); result = ctypes.POINTER(Passwd)(); buffer = ctypes.create_string_buffer(16384)
status = ctypes.CDLL(None).getpwuid_r(os.getuid(), ctypes.byref(entry), buffer, len(buffer), ctypes.byref(result))
print(json.dumps(dict(status=status, home=os.path.realpath(entry.home.decode()) if result else None)))
`], { env: fixture.env, encoding: "utf8" }));
  assert.deepEqual(lookup(), { status: 0, home: fixture.env.TINE_E2E_ACCOUNT_HOME });
  const owners = path.join(fixture.env.TINE_E2E_ACCOUNT_HOME, ".local/state/tine/owners");
  fs.mkdirSync(owners, { recursive: true });
  const lock = path.join(owners, "test.lock");
  // Plausible diagnostic bytes (even for a living PID) are insufficient.
  fs.writeFileSync(lock, `pid ${process.pid}\nowner app\n`);
  assert.throws(() => fixture.verify(), /No app-held kernel owner lease/);
  const stderrFile = `${run}-stderr.log`;
  const stderr = fs.openSync(stderrFile, "w");
  const child = spawn("python3", ["-c", `
import fcntl, os, sys
f = open(sys.argv[1], 'w')
fcntl.flock(f, fcntl.LOCK_EX)
f.write('pid %s\\nowner app\\n' % os.getpid()); f.flush()
print('held', flush=True)
sys.stdin.read()
`, lock], { env: fixture.env, stdio: ["pipe", "pipe", stderr] });
  fs.closeSync(stderr);
  try {
    await once(child.stdout, "data");
    await assertAppLease(fixture);
    // Reusing the fixture does not compile again or duplicate LD_PRELOAD.
    const sibling = accountHomeFixture(run, run, fixture.env);
    try {
      assert.equal(sibling.env.TINE_E2E_ACCOUNT_HOME, fixture.env.TINE_E2E_ACCOUNT_HOME);
      assert.equal(sibling.env.LD_PRELOAD, fixture.env.LD_PRELOAD);
      sibling.verify();
    } finally { sibling.close(); }
    // Driver logs outside the artifacts directory are still inspected.
    fs.writeFileSync(stderrFile, "[tine] owner-endpoint-unavailable\n");
    assert.throws(() => fixture.verify(), /Unexpected owner arbitration fallback/);
    const intentional = accountHomeFixture(run, run, { ...fixture.env,
      TINE_E2E_EXPECT_OWNER_UNAVAILABLE: "exercise explicit fallback opt-out" });
    try { intentional.verify(); }
    finally { intentional.close(); }
  } finally {
    child.stdin.end();
    await once(child, "exit");
    assert.equal(fixture.sample().length, 0, "stale lock bytes after exit are not a held lease");
    fixture.close();
  }
  fs.renameSync(fixture.env.TINE_E2E_ACCOUNT_HOME, `${fixture.env.TINE_E2E_ACCOUNT_HOME}-moved`);
  const missing = lookup();
  assert.notEqual(missing.status, 0, "a missing fixture fails account lookup");
  assert.equal(missing.home, null, "a broken fixture never exposes the real account home");
});
