package page.tine.app

import android.app.Activity
import android.content.Context
import android.content.res.Configuration
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
