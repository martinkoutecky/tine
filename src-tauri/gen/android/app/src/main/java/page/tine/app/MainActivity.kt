package page.tine.app

import android.os.Bundle
import android.os.SystemClock
import android.widget.Toast
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private var lastBlockedBackNoticeAt = Long.MIN_VALUE

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
    // Added after TauriActivity/AppPlugin setup so this is the permanent
    // topmost owner of every physical/gesture Back event.
    onBackPressedDispatcher.addCallback(this, safeBackCallback)
    // `enableEdgeToEdge` follows the OS theme, but Tine has its own persisted
    // light/dark choice. Restore that native appearance before the frontend's
    // first theme sync so system icons never remain light on a light Tine bar.
    SystemBarAppearance.restore(this)
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
    if (now - lastBlockedBackNoticeAt < BACK_NOTICE_THROTTLE_MS) return
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
