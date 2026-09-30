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
        let expected = if key == "identifier" {
            android_namespace.as_ref().unwrap_or(&shipped[key])
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
    println!(
        "cargo:rustc-env=TINE_PRODUCT_NAME={}",
        text(shipped, "productName")
    );
    println!(
        "cargo:rustc-env=TINE_RELEASE_IDENTIFIER={}",
        text(release, "identifier")
    );
    tauri_build::build()
}
