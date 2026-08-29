#!/usr/bin/env bash
set -euo pipefail

# The Android UI lane is intentionally separate from managed-storage runtime
# proof. Each method gets a new app/WebView lifetime: Android's WebView graphics
# teardown has previously poisoned the following instrumentation method, and an
# absent receipt must stay visible rather than becoming a green aggregate job.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
android_root="$repo_root/src-tauri/gen/android"
apk_root="$android_root/app/build/outputs/apk"
artifact_root="${TINE_ANDROID_UI_RUNTIME_ARTIFACT_DIR:-$repo_root/test-results/android-ui-runtime}"
mkdir -p "$artifact_root"

mapfile -t app_apks < <(find "$apk_root" -type f -name '*.apk' ! -name '*androidTest*' -print)
mapfile -t test_apks < <(find "$apk_root" -type f -name '*androidTest.apk' -print)
if [[ ${#app_apks[@]} -ne 1 || ${#test_apks[@]} -ne 1 ]]; then
  printf 'expected one app APK and one instrumentation APK, found %d and %d\n' \
    "${#app_apks[@]}" "${#test_apks[@]}" >&2
  find "$apk_root" -type f -name '*.apk' -print >&2
  exit 1
fi

adb install -r "${app_apks[0]}"
adb install -r "${test_apks[0]}"
# Hosted emulators expose a hardware keyboard by default. Keep the real soft
# keyboard visible as well so WindowInsets/IME assertions exercise the reported
# phone boundary instead of timing out for an emulator configuration reason.
adb shell settings put secure show_ime_with_hard_keyboard 1

{
  printf 'tested_app_apk=%s\n' "${app_apks[0]}"
  printf 'tested_instrumentation_apk=%s\n' "${test_apks[0]}"
  printf '\n===== device =====\n'
  adb shell getprop
  printf '\n===== active WebView =====\n'
  adb shell cmd webviewupdate getCurrentWebViewPackage || true
  printf '\n===== app package =====\n'
  adb shell dumpsys package page.tine.app | grep -E 'versionName=|versionCode=|firstInstallTime=|lastUpdateTime=' || true
} > "$artifact_root/environment.txt"

run_journey() {
  local method="$1"
  local name runner_output runner_log receipt_file failure_file screenshot_file receipts started finished failed status png_signature
  name="${method//./_}"
  name="${name//\#/_}"
  runner_output="$artifact_root/$name.junit.txt"
  runner_log="$artifact_root/$name.logcat.txt"
  receipt_file="$artifact_root/$name.receipt.json"
  failure_file="$artifact_root/$name.failure.json"
  screenshot_file="$artifact_root/$name.png"
  receipts="$artifact_root/$name.receipts.jsonl"

  # A fresh package state makes the test's actual Create a new graph tap a real
  # first-run journey for every method. The fixture lives below app-private data,
  # never a developer machine graph or a desktop-phone-width substitute.
  adb shell am force-stop page.tine.app || true
  adb shell am force-stop page.tine.app.test || true
  adb shell pm clear page.tine.app >/dev/null
  adb shell appops set --uid page.tine.app MANAGE_EXTERNAL_STORAGE allow || true
  adb logcat -c || true

  set +e
  adb shell am instrument -w \
    -e class "page.tine.app.AndroidUiRuntimeTest#$method" \
    page.tine.app.test/androidx.test.runner.AndroidJUnitRunner > "$runner_output" 2>&1
  status=$?
  set -e

  adb logcat -d -v time \
    AndroidRuntime:E DEBUG:V chromium:E TineAndroidUi:I TestRunner:V libc:F '*:S' > "$runner_log" || true
  # A shell screencap after am instrument would not prove the asserted state.
  # The test captures while the menu/selection/topbar is alive and writes
  # beside its JSON receipt; pull those exact in-journey bytes.
  adb exec-out run-as page.tine.app cat "files/android-ui-runtime/$method.png" > "$screenshot_file" || true
  # The log line is convenient in an Actions failure view, while this exact file
  # is the durable DOM/native receipt (large responsive matrices can exceed a
  # single logcat line). Debug instrumentation permits run-as without exposing
  # any user graph or host filesystem data.
  adb exec-out run-as page.tine.app cat "files/android-ui-runtime/$method.json" > "$receipt_file" || true
  if ! adb exec-out run-as page.tine.app cat "files/android-ui-runtime/$method.failure.json" > "$failure_file" 2>/dev/null ||
    ! jq -e . "$failure_file" >/dev/null 2>&1; then
    rm -f "$failure_file"
  fi
  grep -F 'TINE_ANDROID_UI_RUNTIME_RECEIPT ' "$runner_log" > "$receipts" || true
  png_signature="$(od -An -tx1 -N8 "$screenshot_file" 2>/dev/null | tr -d ' \n')"

  started="$(grep -Eo 'run started: [0-9]+ tests?' "$runner_log" | grep -Eo '[0-9]+' | tail -1 || true)"
  finished="$(grep -c 'TestRunner.*finished: ' "$runner_log" || true)"
  failed="$(grep -c 'TestRunner.*failed: ' "$runner_log" || true)"
  {
    printf 'method=%s\n' "$method"
    printf 'instrumentation_exit=%s\n' "$status"
    printf 'runner_started=%s\n' "${started:-none}"
    printf 'runner_finished=%s\n' "$finished"
    printf 'runner_failed=%s\n' "$failed"
    printf 'receipt_file_bytes=%s\n' "$(wc -c < "$receipt_file")"
    printf 'screenshot_file_bytes=%s\n' "$(wc -c < "$screenshot_file")"
    printf 'screenshot_png_signature=%s\n' "${png_signature:-none}"
    printf 'receipt_lines=%s\n' "$(wc -l < "$receipts")"
  } > "$artifact_root/$name.accounting.txt"

  if [[ "$status" -ne 0 ]] || grep -Fq 'FAILURES!!!' "$runner_output" ||
    ! grep -Eq 'OK \(1 test\)' "$runner_output" ||
    [[ "$started" != "1" ]] || [[ "$finished" -ne 1 ]] || [[ "$failed" -ne 0 ]] ||
    [[ ! -s "$receipt_file" ]] ||
    ! jq -e --arg method "$method" '.test == $method and .screenshot == ($method + ".png")' "$receipt_file" >/dev/null 2>&1 ||
    [[ "$png_signature" != "89504e470d0a1a0a" ]] || [[ ! -s "$receipts" ]]; then
    printf 'Android UI runtime method %s is RED; inspect %s\n' "$method" "$artifact_root" >&2
    return 1
  fi
}

overall=0
for method in \
  responsiveChromeFitsPortraitAndLandscapeAtDefault90And110Percent \
  longPressPageReferenceOpensExactlyOnePageActionsMenuWithoutPreviewSelectionOrNavigation \
  initialNativeSelectionShowsMobileToolbarForFirstLineCaretSecondLineHold \
  initialNativeSelectionShowsMobileToolbarForSingleLineHold; do
  if ! run_journey "$method"; then
    overall=1
  fi
done

exit "$overall"
