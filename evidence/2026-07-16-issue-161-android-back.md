# GH #161 A1 — official Android Back and shared root-safe close

## Contract and scope

- Checklist row: **A1 only** on base commit
  `f6f6de13e1180f61ce8d54267d164ffe994eeae7`; all existing dirty #161 work was
  preserved.
- Exactly one official Tauri AppPlugin Back subscription is installed only in
  an Android App WebView. One gesture selects transient, drawer, one browser
  history step, or guarded root close, in that order.
- Android root close and desktop window close use the same graph/session
  persistence transaction. Failed/cancelled confirmation and native-exit
  failure keep the app open and re-arm retry guards.
- Excluded: other checklist rows, E1/E2 scripts, physical-device Back claims,
  commit, deployment, release and GitHub communication.

## Implementation

1. `src/androidBack.ts` owns the Android-only subscription lifecycle and exact
   dispatch ladder. It preserves native fallback before a listener resolves,
   after setup rejection, and after cleanup; cleanup winning either the platform
   or subscription race cannot leave a newly installed listener behind.
2. `src/safeClose.ts` owns the sole bounded close transaction. It blurs/ends the
   edit, bounds `flushAll`, confirms before discard, best-effort flushes the
   session, rejects re-entry, and resets only when close is rejected or the
   native close later fails.
3. `src/App.tsx` injects the official `onBackButtonPress` subscription only
   after `backend().appPlatform()` resolves to `android`; uses exactly one
   `window.history.back()` fallback; and passes the same `safeClose` coordinator
   to Android root close and desktop `onCloseRequested`.
4. Manager review found and this packet fixed a required shared-close neighbor:
   if backend close, `destroy()`, and `close()` all failed, desktop
   `allowClose` previously stayed true. The final catch now resets
   `allowClose`, the safe-close transaction, and `closeInProgress`, preventing a
   later close request from bypassing persistence.
5. The generated `MainActivity.kt` contains neither `handleBackNavigation` nor
   another `OnBackPressedDispatcher` owner.
6. The umbrella regression entry remains `reproduced` but now names
   `src/androidBack.test.ts` and `src/safeClose.test.ts`.

## Focused proof

- `rtk proxy npx vitest run src/androidBack.test.ts src/safeClose.test.ts src/systemBars.test.ts --reporter=verbose`
  — **3 files, 17 tests passed**. Coverage includes transient/drawer/history/root
  order, exactly-one subscription, desktop/iOS exclusion, setup rejection,
  cleanup before platform/subscribe resolution, unregister, source ownership,
  repeated root Back, flush success/failure/timeout, confirm accept/cancel/error,
  session-flush failure, native invoke failure/reset/retry, and desktop guard
  re-arming.
- `rtk proxy npx tsc --noEmit` — **zero errors**.
- `rtk npm run check:regressions` — **passed; 137 UI entries / 108 issues and
  native/index guards valid**.
- Scoped `git diff --check` — **passed**.

Pinned inspected hashes:

```text
b4c4cbc387bb6a2353695ee002a8b95f8008dfa00a55d52976606bf9007976fd  src/App.tsx
4d6446e20630f6aeb991e0adc04ebaf4cb38300f10a7b567e32077fed3cdbb38  src/androidBack.ts
fe377b3fb2264eda0d79185f6ebb2138046ade852de059fc579d84faa0faf29f  src/androidBack.test.ts
c375d8323c3443736552f88ee598a087d1707705a0fece75b329fbdc84b6e183  src/safeClose.ts
a9e8f9a0a0147b5fceef6e661f1678889f5083d9091419d7227f5af58ba9bce5  src/safeClose.test.ts
56c98e5fef17a8499b7fb83166cda02b3a41191e0e55c1a79eeadd7e1a836bd6  src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt
84a8d1768bf40f952c4adceb0d820569738cb61b1d107eb12b4abe1d2f9c9ff9  tests/ui-regressions/catalog.json
```

## Android build boundary

The first literal build attempt used the documented command and failed before
Gradle because this persistent generated project still uses the pre-rename
`page.tine.app` Java path while production config is `page.tine.Tine`:

```text
rtk bash -lc 'source scripts/env.sh && CARGO_PROFILE_DEV_STRIP=symbols ./node_modules/.bin/tauri android build --target aarch64 --apk'
Error Project directory .../java/page/tine/Tine does not exist
```

The retry applied the release workflow's identifier override without mutating
the worktree:

```text
rtk bash -lc "source scripts/env.sh && CARGO_PROFILE_DEV_STRIP=symbols ./node_modules/.bin/tauri android build --config '{\"identifier\":\"page.tine.app\"}' --target aarch64 --apk"
failed to ensure Android environment: Java not found in PATH ... JAVA_HOME ... not set
```

`ANDROID_HOME`, `NDK_HOME`, and `JAVA_HOME` are unset and no Java installation
exists in the ordinary `/usr`, `/opt`, or `/aux` toolchain locations. Per the
manager instruction, no unpinned toolchain was installed. The current generated
Android/Gradle APK build in `.github/workflows/release.yml` is therefore a
**mandatory G1 gate** on the exact candidate; it must compile this
`MainActivity.kt` and Tauri `AppPlugin.kt`. Hardware dispatch remains a
device/emulator-only confidence gap.

## Verdict

**A1 is implementation-complete at the mocked official-listener/source-contract
boundary.** The full Android APK and physical hardware Back are not claimed.
No commit was created.
