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
  private var background: String? = null

  fun restore(activity: Activity) {
    val prefs = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    val fallback = (activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
      Configuration.UI_MODE_NIGHT_YES
    apply(activity, prefs.getBoolean(DARK, fallback), background, persist = false)
  }

  fun apply(activity: Activity, dark: Boolean, background: String? = null, persist: Boolean = true) {
    this.background = background
    if (persist) {
      activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(DARK, dark)
        .apply()
    }
    val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
    val color = SystemBarColor.parse(background)
    val lightIcons = color?.let { SystemBarColor.usesLightIcons(it) } ?: dark
    controller.isAppearanceLightStatusBars = !lightIcons
    controller.isAppearanceLightNavigationBars = !lightIcons
    // The system-bar and cutout insets pad the Activity content root (GH #205),
    // so the strips behind the status and navigation bars are painted by the
    // WINDOW and no longer by the page. GH #693 supplies the computed app
    // background and derives icon contrast from that same colour. Before the
    // first payload, retain GH #467's unqualified mode fallback so Android's
    // own night setting cannot independently change the strip.
    val backing = if (dark) R.color.tine_system_bar_dark else R.color.tine_system_bar_light
    activity.window.setBackgroundDrawable(ColorDrawable(color ?: ContextCompat.getColor(activity, backing)))
  }
}

@TauriPlugin
class SystemBarsPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun setAppearance(invoke: Invoke) {
    val args = invoke.getArgs()
    SystemBarAppearance.apply(activity, args.getBoolean("dark"), args.optString("background", null))
    invoke.resolve()
  }
}
