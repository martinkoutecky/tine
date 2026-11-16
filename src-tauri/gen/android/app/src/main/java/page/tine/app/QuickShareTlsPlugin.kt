package page.tine.app

import android.app.Activity
import android.content.Context
import android.util.Log
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin

/** Tauri waits for this constructor before exposing commands to the WebView.
 * Rust's verifier retains the Application and initializes only once per process.
 * A failure must not stop the app opening the graph: quick share's requests
 * then fail with a TLS error, and the rest of Tine is unaffected. */
@TauriPlugin
class QuickShareTlsPlugin(activity: Activity) : Plugin(activity) {
  init {
    try {
      initVerifier(activity.applicationContext)
    } catch (error: RuntimeException) {
      Log.e("Tine/QuickShareTls", "Certificate verifier init failed; quick share will not connect", error)
    } catch (error: UnsatisfiedLinkError) {
      Log.e("Tine/QuickShareTls", "Certificate verifier init missing; quick share will not connect", error)
    }
  }

  private external fun initVerifier(context: Context)
}
