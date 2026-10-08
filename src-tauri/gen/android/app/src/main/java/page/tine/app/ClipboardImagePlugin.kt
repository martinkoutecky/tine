package page.tine.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.BitmapFactory
import android.util.Base64
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileOutputStream

// Image copy for Android (GH #654). tauri-plugin-clipboard-manager has no mobile
// image support, so the PNG from the page is staged here as a `tine_clip_*.png`
// file directly in the app cache and published as a content:// URI through the
// app's FileProvider (the Android-sanctioned way to put an image on the
// clipboard). The pasting app, this WebView included, reads the image through
// that URI.
private const val MAX_CLIP_BYTES = 64L * 1024L * 1024L
private const val MAX_CLIP_PIXELS = 64L * 1024L * 1024L

@TauriPlugin
class ClipboardImagePlugin(private val activity: Activity) : Plugin(activity) {
  private val manager: ClipboardManager =
    activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

  @Command
  fun copyImage(invoke: Invoke) {
    var staged: File? = null
    try {
      val bytes = Base64.decode(invoke.getArgs().getString("bytesB64"), Base64.DEFAULT)
      if (bytes.isEmpty() || bytes.size > MAX_CLIP_BYTES) {
        invoke.reject("Image is empty or exceeds the 64 MiB limit")
        return
      }
      val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
      BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
      if (bounds.outWidth <= 0 || bounds.outHeight <= 0 ||
        bounds.outWidth.toLong() * bounds.outHeight.toLong() > MAX_CLIP_PIXELS
      ) {
        invoke.reject("Not a supported image")
        return
      }
      // The new clip replaces the old one, so retire the previous staged image:
      // repeated copies cannot grow the cache.
      activity.cacheDir.listFiles()
        ?.filter { it.name.startsWith("tine_clip_") && it.name.endsWith(".png") }
        ?.forEach { it.delete() }
      val file = File.createTempFile("tine_clip_", ".png", activity.cacheDir)
      staged = file
      FileOutputStream(file, false).use { out ->
        out.write(bytes)
        out.fd.sync()
      }
      val uri = FileProvider.getUriForFile(
        activity, "${activity.packageName}.fileprovider", file
      )
      manager.setPrimaryClip(ClipData.newUri(activity.contentResolver, "Tine image", uri))
      invoke.resolve()
    } catch (ex: Exception) {
      staged?.delete()
      invoke.reject(ex.message ?: "Failed to copy the image")
    }
  }
}
