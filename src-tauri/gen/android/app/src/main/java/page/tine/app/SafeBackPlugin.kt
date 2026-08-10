package page.tine.app

import android.app.Activity
import android.webkit.WebView
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * One deliberately narrow bridge between Android's permanent Back owner and
 * the frontend dispatcher. Tauri's built-in AppPlugin falls through to WebView
 * history or Activity finish when it has no JS listener; that is never an
 * acceptable fallback while Tine has not verified managed-storage shutdown.
 */
internal object SafeBackBridge {
  private var plugin: SafeBackPlugin? = null

  fun install(plugin: SafeBackPlugin) {
    this.plugin = plugin
  }

  fun dispatchIfReady(): Boolean = plugin?.dispatchIfReady() ?: false

  fun clear() {
    plugin?.clearWebView()
    plugin = null
  }
}

@TauriPlugin
class SafeBackPlugin(private val activity: Activity) : Plugin(activity) {
  private var webView: WebView? = null

  init {
    SafeBackBridge.install(this)
  }

  override fun load(webView: WebView) {
    this.webView = webView
  }

  /**
   * Dispatch only to the listener the frontend explicitly registered through
   * addPluginListener("safe-back", "android-safe-back", ...). Returning false
   * means the activity must consume Back itself; this method intentionally has
   * no WebView-history or activity-finish fallback.
   */
  fun dispatchIfReady(): Boolean {
    val loadedWebView = webView ?: return false
    if (!hasListener("android-safe-back")) return false
    trigger("android-safe-back", JSObject().apply {
      put("canGoBack", loadedWebView.canGoBack())
    })
    return true
  }

  fun clearWebView() {
    webView = null
  }
}
