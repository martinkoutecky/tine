package page.tine.app

import android.os.Bundle
import android.os.SystemClock
import android.widget.Toast
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  /** `elapsedRealtime` of the last notice, or null while none has been shown.
   * NOT a sentinel Long: `now - Long.MIN_VALUE` overflows to a negative value,
   * which is below any throttle, so the first notice suppressed itself and the
   * stamp was never written — the notice could never appear at all. */
  private var lastBlockedBackNoticeAt: Long? = null

  private val safeBackCallback = object : OnBackPressedCallback(true) {
    override fun handleOnBackPressed() {
      // Never disable this callback or delegate to the lower AppPlugin: its
      // no-listener fallback can navigate WebView history or finish Activity
      // before managed storage has proved a clean process stop.
      if (!SafeBackBridge.dispatchIfReady()) showBlockedBackNotice()
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android WebView 124 on API 35 reports CSS env(safe-area-inset-*) as zero
    // even in viewport-fit=cover. Apply the actual system-bar/cutout insets to
    // the Activity content root so the WebView viewport itself starts below the
    // status bar and ends above navigation. Returning the unconsumed insets is
    // intentional: descendants still need IME visibility for editor behavior.
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val safe = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
      )
      view.setPadding(safe.left, safe.top, safe.right, safe.bottom)
      insets
    }
    ViewCompat.requestApplyInsets(content)
    // Earliest possible owner: until Tauri is up there is no other callback at
    // all, and the platform default for an unhandled Back is finish(). This
    // registration is NOT sufficient by itself — see takeBackOwnership.
    onBackPressedDispatcher.addCallback(this, safeBackCallback)
    // `enableEdgeToEdge` follows the OS theme, but Tine has its own persisted
    // light/dark choice. Restore that native appearance before the frontend's
    // first theme sync so system icons never remain light on a light Tine bar.
    SystemBarAppearance.restore(this)
  }

  /**
   * Move Tine's Back owner back to the top of the dispatcher.
   *
   * OnBackPressedDispatcher keeps its callbacks in an ArrayDeque and gives a
   * gesture to the LAST enabled one that was added, so "topmost owner" means
   * "most recently added", not "added first".
   *
   * Registering in onCreate can never win that race. tao's Android entry point
   * spawns the Rust `main` on its own thread from `Rust.create()`, so Tauri
   * builds the app — and constructs the Kotlin plugins, each of which may add
   * its own callback from its constructor — only after this Activity's
   * onCreate has already returned. Tauri's core AppPlugin does exactly that,
   * and its callback is always enabled, so it landed ABOVE this one.
   *
   * AppPlugin's handler takes the no-listener branch (Tine registers
   * "android-safe-back" on its own plugin, never "back-button" on Tauri's) and
   * calls WebView.goBack() whenever the WebView has history. The mobile router
   * pushes one history entry per navigation, so after the user's first
   * navigation every Back silently popped a route BEHIND whatever was on
   * screen: an open modal never saw the gesture and never closed. Before that
   * first navigation canGoBack() is false, AppPlugin re-dispatches, and Back
   * behaves correctly — which is why this looked intermittent rather than
   * broken.
   *
   * SafeBackPlugin calls this once it is constructed and again when its WebView
   * loads, both of which are strictly after every plugin registered so far.
   */
  internal fun takeBackOwnership() {
    safeBackCallback.remove()
    onBackPressedDispatcher.addCallback(this, safeBackCallback)
  }

  override fun onResume() {
    super.onResume()
    SystemBarAppearance.restore(this)
  }

  override fun onDestroy() {
    SafeBackBridge.clear()
    super.onDestroy()
  }

  private fun showBlockedBackNotice() {
    val now = SystemClock.elapsedRealtime()
    val previous = lastBlockedBackNoticeAt
    if (previous != null && now - previous < BACK_NOTICE_THROTTLE_MS) return
    lastBlockedBackNoticeAt = now
    Toast.makeText(
      this,
      "Tine is still starting or retrying a safe close. Please wait, then try Back again.",
      Toast.LENGTH_SHORT,
    ).show()
  }

  private companion object {
    const val BACK_NOTICE_THROTTLE_MS = 2_000L
  }
}
