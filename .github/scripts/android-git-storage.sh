#!/usr/bin/env bash
# Runs the app crate's git tests on a booted emulator (ci.yml,
# android-git-storage): first with the graph on shared storage, then the
# autocommit and seen suites on app-private storage. Each run must print its
# libtest success line; a missing or crashed run fails.
set -euo pipefail

bin="$1"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$repo_root/artifacts/android-git-storage"
mkdir -p "$out"
trap 'adb logcat -d -v threadtime > "$out/logcat.txt" 2>&1 || true' EXIT

device=/data/local/tmp/tine-git-tests
shared=/sdcard/Documents/tine-git-storage
adb push "$bin" "$device/tine_lib_tests" > /dev/null
adb shell "chmod 755 $device/tine_lib_tests && mkdir -p $device/tmp $shared"
adb shell getprop ro.build.version.sdk > "$out/sdk.txt"
adb shell mount > "$out/mount.txt" || true

run() {
  local name="$1" env="$2"
  shift 2
  set +e
  timeout 1200s adb shell "cd $device && env TMPDIR=$device/tmp $env ./tine_lib_tests $* --test-threads=1 --nocapture" \
    2>&1 | tee "$out/$name.txt"
  set -e
  if ! grep -Eq '^test result: ok\. [1-9][0-9]* passed; 0 failed' "$out/$name.txt"; then
    echo "$name: the tests did not all pass" >&2
    exit 1
  fi
}

run shared-storage "TINE_HOST_FS_ROOT=$shared TINE_EXPECT_FUSE=1" owner::git_autocommit::mobile_tests
for line in 'GIT-STORAGE result=PASS' 'GIT-KILL result=PASS'; do
  grep -q "$line" "$out/shared-storage.txt" || { echo "missing: $line" >&2; exit 1; }
done
run private-storage "" owner::git_autocommit owner::seen
