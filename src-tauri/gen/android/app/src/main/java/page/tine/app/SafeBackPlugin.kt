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
 * acceptable fallback while Tine has not verified a clean shutdown.
 */
internal object SafeBackBridge {
  private var plugin: SafeBackPlugin? = null

  /** Every Back gesture that reached Tine's owner, whether or not the frontend
   * listener was ready. Ownership and readiness are separate failures and the
   * runtime test has to be able to tell them apart. */
  internal var gesturesReceived: Int = 0
    private set

  /** Gestures actually handed to the frontend listener. Ownership and
   * DELIVERY are different failures: an ACL that refuses
   * `plugin:safe-back|registerListener` leaves this at zero while every
   * gesture still reaches the owner, which is exactly how Back became a
   * silent no-op on a real device while the emulator run stayed green. */
  internal var dispatchesDelivered: Int = 0
    private set

  fun install(plugin: SafeBackPlugin) {
    this.plugin = plugin
  }

  fun dispatchIfReady(): Boolean {
    gesturesReceived += 1
    val delivered = plugin?.dispatchIfReady() ?: false
    if (delivered) dispatchesDelivered += 1
    return delivered
  }

  /** The live WebView, for the instrumentation that has to arrange real
   * WebView history before it can observe who owns Back. */
  internal fun webViewForTest(): WebView? = plugin?.webViewForTest()

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
    // Claiming Back ownership belongs HERE, not in MainActivity.onCreate: see
    // MainActivity.takeBackOwnership for why onCreate is always too early.
    // Tauri registers its core plugins (AppPlugin among them) before it
    // initializes any user plugin, so this constructor is strictly later than
    // AppPlugin's, and the dispatcher hands Back to the later registration.
    claimBackOwnership()
  }

  override fun load(webView: WebView) {
    this.webView = webView
    // `load` runs once the WebView exists, after every plugin registered up to
    // that point has been constructed. Re-claiming here keeps the invariant
    // ("Tine's owner is the newest callback on the dispatcher") true for a
    // plugin added after this one, instead of only for today's plugin list.
    claimBackOwnership()
  }

  private fun claimBackOwnership() {
    (activity as? MainActivity)?.takeBackOwnership()
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

  internal fun webViewForTest(): WebView? = webView

  fun clearWebView() {
    webView = null
  }
}
