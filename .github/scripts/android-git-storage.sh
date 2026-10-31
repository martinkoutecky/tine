#!/usr/bin/env bash
# Runs the page host's save cases (tine-store, GH #811) and the app crate's
# git tests on a booted emulator (ci.yml, android-git-storage): first with the
# graph on shared storage, then the autocommit and seen suites on app-private
# storage. Each run must print its
# libtest success line; a missing or crashed run fails the job after every
# run has had its turn.
set -euo pipefail

bin="$1"
store_bin="$2"
store_lib_bin="$3"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$repo_root/artifacts/android-git-storage"
mkdir -p "$out"
trap 'adb logcat -d -v threadtime > "$out/logcat.txt" 2>&1 || true' EXIT

device=/data/local/tmp/tine-git-tests
shared=/sdcard/Documents/tine-git-storage
# GH #811: emulators leave MediaProvider's Android FUSE passthrough off
# (persist.sys.fuse.passthrough.enable unset); devices commonly turn it on.
# Its reads copy the backing file's mtime/ctime into the FUSE inode
# (fs/fuse/passthrough.c fuse_file_accessed), so test with it on as well.
if [ "${TINE_FUSE_PASSTHROUGH:-}" = 1 ]; then
  adb root > /dev/null
  adb wait-for-device
  adb shell setprop persist.sys.fuse.passthrough.enable true
  adb reboot
  adb wait-for-device
  until [ "$(adb shell getprop sys.boot_completed | tr -d '\r')" = 1 ]; do sleep 2; done
fi
adb push "$bin" "$device/tine_lib_tests" > /dev/null
adb push "$store_bin" "$device/tine_store_integration" > /dev/null
adb push "$store_lib_bin" "$device/tine_store_lib" > /dev/null
adb shell "chmod 755 $device/tine_lib_tests $device/tine_store_integration $device/tine_store_lib && mkdir -p $device/tmp $shared/accept"
# Just after boot MediaProvider may still refuse writes in shared storage
# (EPERM on the first file, run 40762656695 API 33): wait until a file of the
# kinds the tests write can be created and removed.
ready=0
for i in $(seq 1 60); do
  if adb shell "mkdir -p $shared/probe-$i && echo p > $shared/probe-$i/p.edn && echo p > $shared/probe-$i/p.md && rm -r $shared/probe-$i" 2> /dev/null; then
    ready=1
    break
  fi
  sleep 2
done
[ "$ready" = 1 ] || echo "shared storage never accepted a write" >&2
adb shell getprop ro.build.version.sdk > "$out/sdk.txt"
adb shell mount > "$out/mount.txt" || true
# Which FUSE features MediaProvider can use on this kernel (GH #811: Android 16
# MediaProvider adds upstream FUSE passthrough when fuse_passthrough reads
# 'supported'), and what it chose.
adb shell 'for f in /sys/fs/fuse/features/*; do echo "$f: $(cat "$f")"; done; uname -r; getprop | grep -i fuse' \
  > "$out/fuse.txt" 2>&1 || true
adb logcat -d -s FuseDaemon:* >> "$out/fuse.txt" 2>&1 || true
cat "$out/fuse.txt"
# Who the tests run as, and whom shared storage says owns its files (libgit2's
# owner check, git_run::VERIFY_OWNER).
adb shell "id; stat -c '%u:%g %A %n' $shared $device" > "$out/owners.txt" || true
cat "$out/owners.txt"

run() {
  local name="$1" env="$2"
  shift 2
  run_bin tine_lib_tests "$name" "$env" "$@"
}
run_bin() {
  local exe="$1" name="$2" env="$3"
  shift 3
  set +e
  timeout 1200s adb shell "cd $device && env TMPDIR=$device/tmp $env ./$exe $* --test-threads=1 --nocapture" \
    2>&1 | tee "$out/$name.txt"
  set -e
  if ! grep -Eq '^test result: ok\. [1-9][0-9]* passed; 0 failed' "$out/$name.txt"; then
    echo "$name: the tests did not all pass" >&2
    failed=1
  fi
}
failed=0

# Page saves through the page host publish on shared storage (GH #811): the
# app data stays app-private, as on a phone.
run_bin tine_store_integration store-shared-storage "TINE_HOST_FS_ROOT=$shared" gh811_graph_volume_save
# The publication-read cases of the post-sync check, on the device volume.
run_bin tine_store_lib store-publication-read "TINE_HOST_FS_ROOT=$shared" page_host::production_tests::gh811_
# F-M3S-1: the supported-volume acceptance of the post-sync stamp check.
run_bin tine_store_lib store-acceptance "TINE_STORAGE_ACCEPT_ROOT=$shared/accept" storage_acceptance_tests --ignored
# The passthrough run is for the page saves only.
[ "${TINE_FUSE_PASSTHROUGH:-}" = 1 ] && exit "$failed"
run shared-storage "TINE_HOST_FS_ROOT=$shared TINE_EXPECT_FUSE=1" owner::git_autocommit::mobile_tests
for line in 'GIT-STORAGE result=PASS' 'GIT-KILL result=PASS'; do
  grep -q "$line" "$out/shared-storage.txt" || { echo "missing: $line" >&2; failed=1; }
done
# The OS ending the app is a race: give it more kills.
for n in 1 2 3; do
  run "shared-storage-kill-$n" "TINE_HOST_FS_ROOT=$shared TINE_EXPECT_FUSE=1" \
    --exact owner::git_autocommit::mobile_tests::a_commit_or_push_the_os_ends_leaves_the_repository_whole
done
run private-storage "" owner::git_autocommit owner::seen
exit "$failed"
