//! Android's platform verifier needs the app's class loader and application
//! context before reqwest constructs a verifier. Linux, macOS, Windows and iOS
//! use their existing transports without this JNI bootstrap.

use jni::{objects::JObject, EnvUnowned};
use tauri::{plugin::TauriPlugin, Runtime};

// Called synchronously by the Kotlin plugin constructor. Upstream's OnceCell
// initializes exactly once per process; retain the application, never Activity.
// JNI errors throw a RuntimeException that the Kotlin constructor logs: the
// app still opens, and quick share then fails its requests with a TLS error.
#[jni::jni_mangle("page.tine.app.QuickShareTlsPlugin")]
pub fn init_verifier<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    context: JObject<'local>,
) {
    env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("quick-share-tls")
        .setup(|_app, api| {
            api.register_android_plugin("page.tine.app", "QuickShareTlsPlugin")?;
            Ok(())
        })
        .build()
}
