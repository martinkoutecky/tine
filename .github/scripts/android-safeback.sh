#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
apk_root="$repo_root/src-tauri/gen/android/app/build/outputs/apk"
artifact_root="$repo_root/artifacts/safeback-ci"
mkdir -p "$artifact_root"

# Preserve device evidence before the emulator action shuts it down. A failed
# install, assertion, timeout or native crash must retain its original exit code.
collect_device_evidence() {
  adb logcat -d -v threadtime > "$artifact_root/logcat.txt" 2>&1 || true
  adb shell dumpsys package page.tine.beta > "$artifact_root/package.txt" 2>&1 || true
  adb shell dumpsys webviewupdate > "$artifact_root/webview.txt" 2>&1 || true
}
trap collect_device_evidence EXIT

mapfile -t app_apks < <(find "$apk_root" -type f -name '*-debug.apk' ! -path '*/androidTest/*')
mapfile -t test_apks < <(find "$apk_root" -type f -name '*-debug-androidTest.apk')
if [[ ${#app_apks[@]} -ne 1 || ${#test_apks[@]} -ne 1 ]]; then
  printf 'expected one debug app APK and one instrumentation APK, found %d and %d\n' \
    "${#app_apks[@]}" "${#test_apks[@]}" >&2
  exit 1
fi
cp "${app_apks[0]}" "$artifact_root/tested-app.apk"
cp "${test_apks[0]}" "$artifact_root/tested-instrumentation.apk"
(
  cd "$artifact_root"
  sha256sum tested-app.apk tested-instrumentation.apk > tested-apks.sha256
)
adb install -r "$artifact_root/tested-app.apk"
adb install -r "$artifact_root/tested-instrumentation.apk"
adb logcat -c

# am instrument can exit zero on a failed JUnit test or a crashed app. Require
# the explicit one-test success summary; a missing test is a failure too.
timeout 180s adb shell am instrument -w \
  -e class page.tine.app.SafeBackOwnershipTest \
  page.tine.beta.test/androidx.test.runner.AndroidJUnitRunner \
  | tee "$artifact_root/instrumentation.txt"
if ! grep -Eq '^OK \(1 test\)' "$artifact_root/instrumentation.txt" ||
  grep -Eq 'FAILURES!!!|INSTRUMENTATION_FAILED|Process crashed' "$artifact_root/instrumentation.txt"; then
  echo 'SafeBack ownership instrumentation did not pass exactly one test' >&2
  exit 1
fi
