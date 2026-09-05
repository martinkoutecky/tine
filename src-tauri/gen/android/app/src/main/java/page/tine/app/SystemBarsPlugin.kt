package page.tine.app

import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import android.graphics.drawable.ColorDrawable
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

internal object SystemBarAppearance {
  private const val PREFS = "tine-native-appearance"
  private const val DARK = "dark"

  fun restore(activity: Activity) {
    val prefs = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    val fallback = (activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
      Configuration.UI_MODE_NIGHT_YES
    apply(activity, prefs.getBoolean(DARK, fallback), persist = false)
  }

  fun apply(activity: Activity, dark: Boolean, persist: Boolean = true) {
    if (persist) {
      activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(DARK, dark)
        .apply()
    }
    val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
    controller.isAppearanceLightStatusBars = !dark
    controller.isAppearanceLightNavigationBars = !dark
    // Since 0.6.981 the system-bar and cutout insets pad the Activity content
    // root, so the strips behind the status and navigation bars are painted by
    // the WINDOW and no longer by the page. The window background comes from
    // values/ vs values-night, which follows the ANDROID night setting -- while
    // the icon appearance two lines above follows TINE's own theme. A user with
    // Tine in dark mode on a phone still in light mode therefore got light-mode
    // icons, which are white, on the light-mode strip, which is also white: an
    // empty notification bar (GH #467). ONE authority now paints both. The
    // colors are read unqualified on purpose, so the Android night setting
    // cannot re-enter through the resource resolver.
    val backing = if (dark) R.color.tine_system_bar_dark else R.color.tine_system_bar_light
    activity.window.setBackgroundDrawable(ColorDrawable(ContextCompat.getColor(activity, backing)))
  }
}

@TauriPlugin
class SystemBarsPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun setAppearance(invoke: Invoke) {
    SystemBarAppearance.apply(activity, invoke.getArgs().getBoolean("dark"))
    invoke.resolve()
  }
}
