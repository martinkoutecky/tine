package page.tine.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android WebView 124 on API 35 reports CSS env(safe-area-inset-*) as zero
    // even in viewport-fit=cover. Apply the actual system-bar/cutout insets to
    // the Activity content root so the WebView viewport itself starts below the
    // status bar and ends above navigation OR the keyboard (GH #205; master
    // d5412929a). Edge-to-edge WebView can leave visualViewport unchanged under
    // IME occlusion; this native viewport owner must exclude it too. This is the
    // sole inset owner on Android: src/systemInsets.ts zeroes the CSS tokens
    // here. Return unconsumed insets so descendants can still observe IME
    // visibility without introducing a second geometry owner.
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val safe = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
      )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(safe.left, safe.top, safe.right, maxOf(safe.bottom, ime.bottom))
      insets
    }
    ViewCompat.requestApplyInsets(content)
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
