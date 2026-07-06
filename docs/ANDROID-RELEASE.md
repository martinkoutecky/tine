# Android release signing

Debug APKs are signed with the shared, publicly-known Android *debug* key. On a
phone that triggers Play Protect's harsh "unsafe app / unknown developer" wall
(the one where **Install anyway** is buried). A **release build signed with our
own key** removes the debug-cert stigma, gives Tine a stable identity, and lets
installs update in place. It's also required for F-Droid and the Play Store.

The signing credentials are read from `src-tauri/gen/android/keystore.properties`,
which is **gitignored** — no key or password is ever committed. Without that file,
`build.gradle.kts` still builds, but the release APK comes out unsigned.

## One-time setup (do this once; you own the key)

**1. Generate a release keystore** (keep it OUTSIDE the repo; back it up somewhere
safe — if you lose it you can never ship an update under the same identity, and if
it leaks someone can publish malicious "Tine updates"):

```sh
mkdir -p ~/.android-keys
keytool -genkeypair -v \
  -keystore ~/.android-keys/tine-release.jks \
  -alias tine \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -dname "CN=Tine, O=Tine, C=CZ"
```

It prompts for a keystore password (and a key password — you can reuse the same).
Remember them; they go in the properties file below.

**2. Create `src-tauri/gen/android/keystore.properties`** (gitignored) with:

```properties
storeFile=/home/koutecky/.android-keys/tine-release.jks
storePassword=YOUR_STORE_PASSWORD
keyAlias=tine
keyPassword=YOUR_KEY_PASSWORD
```

Use an **absolute** `storeFile` path. That's it — the build picks it up
automatically.

## Building a signed release

With the Android env exported (see `subagent-tasks` / the build memo), from the
repo root:

```sh
# Signed release APK (sideload / F-Droid):
CARGO_PROFILE_DEV_STRIP=symbols npx tauri android build --target aarch64 --apk
#   → src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk

# Signed release AAB (Play Store, later):
npx tauri android build --target aarch64 --aab
```

Note: **no `--debug`** — that's what makes it a release build. Verify it's signed
with your key (not the debug key):

```sh
$ANDROID_HOME/build-tools/35.0.0/apksigner verify --print-certs \
  src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

The printed certificate DN should be `CN=Tine, …`, not `CN=Android Debug`.

## Signing off-machine (when the build host must not hold the key)

The build sandbox on the university machine deliberately does **not** hold the
private key (its home is world-readable NFS). So the key lives only on a private
machine, and we sign there:

1. **On the build host** — build an *unsigned* release APK. Because
   `build.gradle.kts` gates signing on the keystore *file* existing (not just
   `keystore.properties`), a host without the `.jks` produces
   `…/apk/universal/release/app-universal-release-unsigned.apk` cleanly. Ensure
   it's zipaligned before delivery:
   ```sh
   $ANDROID_HOME/build-tools/35.0.0/zipalign -c -v 4 app-universal-release-unsigned.apk  # verify
   ```
   (Gradle release packaging already aligns it; the check should pass.)

2. **On the private machine that has the key** — sign with the self-contained
   `apksigner.jar` (needs only a JDK, which you have from `keytool`):
   ```sh
   java -jar apksigner.jar sign \
     --ks ~/.android-keys/tine-release.jks --ks-key-alias tine \
     --out tine-release-signed.apk app-universal-release-unsigned.apk
   ```
   It prompts for the keystore password. Verify:
   ```sh
   java -jar apksigner.jar verify --print-certs tine-release-signed.apk   # → CN=Tine
   ```
   Install `tine-release-signed.apk`. The private key never touches the build host.

## Notes

- Minification (R8/ProGuard) is currently **off** for release: Tauri mobile
  plugins (including our folder picker) are resolved reflectively by class name,
  which R8 can strip without a device to catch it. Re-enable with vetted
  keep-rules once a minified build can be tested on hardware.
- Play Store uses **Play App Signing**: Google holds the final signing key and
  your release key becomes the *upload* key. F-Droid signs with its own key (or
  builds reproducibly). Either way, keep this keystore safe.
