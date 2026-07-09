import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Release signing: credentials live in gen/android/keystore.properties, which is
// gitignored (never commit the keystore or its passwords). Expected keys:
// storeFile (absolute path to the .jks), storePassword, keyAlias, keyPassword.
// See docs/ANDROID-RELEASE.md.
//
// We gate on the KEYSTORE FILE actually existing, not merely on the properties
// file — so a machine that has the properties but not the key (e.g. this build
// sandbox, where the private key deliberately never lives) still builds a valid
// *unsigned* release APK to be signed off-machine, instead of failing at the
// validateSigning task.
val keystorePropertiesFile = rootProject.file("keystore.properties")
val keystoreProperties = Properties().apply {
    if (keystorePropertiesFile.exists()) {
        keystorePropertiesFile.inputStream().use { load(it) }
    }
}
val releaseStoreFile = keystoreProperties.getProperty("storeFile")?.let { file(it) }
val hasReleaseKey = releaseStoreFile != null && releaseStoreFile.exists()

android {
    compileSdk = 36
    // Reproducible build (F-Droid #reproducible): pin the NDK to the same version the
    // F-Droid recipe declares (`ndk: 26.3.11579264`, clang/lld 17) so AGP never
    // resolves or downloads a newer NDK. Paired with removing other NDKs on the CI
    // runner (release.yml) so the tauri-CLI Rust link also uses exactly this one.
    ndkVersion = "26.3.11579264"
    namespace = "page.tine.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "page.tine.app"
        minSdk = 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        create("release") {
            if (hasReleaseKey) {
                storeFile = releaseStoreFile
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // Sign with the release key when the keystore file is actually present;
            // otherwise produce an unsigned release APK to be signed off-machine
            // (see docs/ANDROID-RELEASE.md) — the debug build stays the fallback.
            if (hasReleaseKey) {
                signingConfig = signingConfigs.getByName("release")
            }
            // Minification is OFF for now: the folder-picker plugin (and Tauri's
            // mobile plugins generally) are resolved reflectively by class name, so
            // ProGuard/R8 can strip or rename them and break the app at runtime —
            // not something we can catch without a device. Re-enable with vetted
            // keep-rules once we can test a minified build on hardware.
            isMinifyEnabled = false
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")