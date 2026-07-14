package page.tine.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  // TauriActivity disables the OnBackPressed callback (handleBackNavigation=false),
  // so the system back gesture always finishes the activity (exits the app).
  // Re-enable it: the WryActivity callback then routes back to WebView.goBack()
  // when there's history, exiting only at the root. Tine's router publishes that
  // history via the mobile History-API bridge (see src/router.ts).
  override val handleBackNavigation: Boolean = true

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // `enableEdgeToEdge` follows the OS theme, but Tine has its own persisted
    // light/dark choice. Restore that native appearance before the frontend's
    // first theme sync so system icons never remain light on a light Tine bar.
    SystemBarAppearance.restore(this)
  }

  override fun onResume() {
    super.onResume()
    SystemBarAppearance.restore(this)
  }
}
