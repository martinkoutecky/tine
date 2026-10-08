package page.tine.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File

// Image copy for Android (GH #654). tauri-plugin-clipboard-manager has no mobile
// image support, so Rust (android_clipboard.rs) stages the PNG as a
// `tine_clip_*.png` file directly in the app cache and this plugin publishes it
// as a content:// URI through the app's FileProvider. The pasting app, this
// WebView included, reads the image through that URI.
@TauriPlugin
class ClipboardImagePlugin(private val activity: Activity) : Plugin(activity) {
  private val manager: ClipboardManager =
    activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

  @Command
  fun copyImage(invoke: Invoke) {
    try {
      val file = File(invoke.getArgs().optString("path", "")).canonicalFile
      // Only a staged clipboard image directly inside the app cache may be
      // published: the path crosses the Rust bridge, so it is checked again here.
      if (file.parentFile != activity.cacheDir.canonicalFile ||
        !file.name.startsWith("tine_clip_") || !file.name.endsWith(".png") ||
        !file.isFile || file.length() <= 0L
      ) {
        invoke.reject("Not a staged clipboard image")
        return
      }
      val uri = FileProvider.getUriForFile(
        activity, "${activity.packageName}.fileprovider", file
      )
      manager.setPrimaryClip(ClipData.newUri(activity.contentResolver, "Tine image", uri))
      invoke.resolve()
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "Failed to copy the image")
    }
  }
}
