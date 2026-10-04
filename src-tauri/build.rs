// The app identity switch is `app-identity.json` (docs/app-identity.md). The
// Rust code reads the shipped identity from these compile-time variables and
// never spells an identifier; the build refuses a tauri.conf.json that
// disagrees with the switch (`node scripts/set-app-identity.mjs <ship>`
// rewrites every derived place).
fn main() {
    println!("cargo:rerun-if-changed=app-identity.json");
    let read = |file: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(file).expect(file)).expect(file)
    };
    let switch = read("app-identity.json");
    let ship = switch["ship"].as_str().expect("app-identity.json: ship");
    let shipped = &switch["identities"][ship];
    let release = &switch["identities"]["release"];
    let conf = read("tauri.conf.json");
    let android = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android");
    let android_namespace = if android {
        println!("cargo:rerun-if-changed=gen/android/app/build.gradle.kts");
        let gradle =
            std::fs::read_to_string("gen/android/app/build.gradle.kts").expect("Android Gradle");
        let field = |key: &str| {
            gradle
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(&format!("{key} = \""))
                        .and_then(|v| v.strip_suffix('"'))
                })
                .expect(key)
                .to_owned()
        };
        assert_eq!(
            field("applicationId"),
            shipped["androidApplicationId"]
                .as_str()
                .expect("androidApplicationId"),
            "Android applicationId disagrees with the identity switch"
        );
        Some(serde_json::Value::String(field("namespace")))
    } else {
        None
    };
    for key in ["identifier", "productName"] {
        let expected = if key == "identifier" && android_namespace.as_ref() == Some(&conf[key]) {
            // Direct cargo checks keep the canonical desktop config. The
            // Android CLI instead selects the generated Kotlin source namespace.
            android_namespace.as_ref().expect("Android namespace")
        } else {
            &shipped[key]
        };
        assert_eq!(
            &conf[key], expected,
            "tauri.conf.json `{key}` disagrees with app-identity.json (ship = {ship}); \
             run `node scripts/set-app-identity.mjs {ship}`"
        );
    }
    let text = |value: &serde_json::Value, key: &str| value[key].as_str().expect(key).to_owned();
    println!(
        "cargo:rustc-env=TINE_APP_IDENTIFIER={}",
        text(shipped, "identifier")
    );
    // The WebProcess loads this small module separately from the UI binary.
    // Reuse rustc, with no new crate or system build dependency.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        let referer = format!("https://{}/", text(shipped, "identifier").to_ascii_lowercase());
        println!("cargo:rustc-env=TINE_YOUTUBE_REFERER={referer}");
        println!("cargo:rerun-if-changed=src/youtube_web_extension.rs");
        let mut compiler = std::process::Command::new(std::env::var_os("RUSTC").expect("RUSTC"));
        // Cargo selects this linker for Linux cross builds too.
        if let Some(linker) = std::env::var_os("RUSTC_LINKER") {
            compiler.arg("-C").arg(format!("linker={}", linker.to_string_lossy()));
        }
        let status = compiler
            .args([
                "src/youtube_web_extension.rs",
                "--crate-type=cdylib",
                "--edition=2021",
                "-Copt-level=s",
                "-Cpanic=abort",
                "-Cstrip=symbols",
                "--target",
            ])
            .arg(std::env::var_os("TARGET").expect("TARGET"))
            .arg("-o")
            .arg(
                std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
                    .join("libtine_youtube.so"),
            )
            .env("TINE_YOUTUBE_REFERER", referer)
            .status()
            .expect("compile YouTube WebProcess extension");
        assert!(status.success(), "YouTube WebProcess extension compilation failed");
    }
    println!(
        "cargo:rustc-env=TINE_PRODUCT_NAME={}",
        text(shipped, "productName")
    );
    println!(
        "cargo:rustc-env=TINE_RELEASE_IDENTIFIER={}",
        text(release, "identifier")
    );
    // `safe-back` is an INLINED plugin (src/android_safe_back.rs): it lives in
    // this crate, so nothing generates an ACL manifest for it unless this build
    // script does. Without one, `plugin:safe-back|registerListener` is refused
    // before it reaches Android, the frontend's Back listener never registers,
    // and the native owner consumes every gesture with nowhere to send it —
    // invisible from Rust and from the emulator alike (master 61a663291 line).
    tauri_build::try_build(
        tauri_build::Attributes::new().plugin(
            "safe-back",
            tauri_build::InlinedPlugin::new()
                .commands(&["registerListener", "removeListener"])
                .default_permission(tauri_build::DefaultPermissionRule::AllowAllCommands),
        ),
    )
    .expect("failed to run tauri-build");
}
