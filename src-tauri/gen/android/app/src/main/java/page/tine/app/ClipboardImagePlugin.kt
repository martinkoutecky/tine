package page.tine.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.BitmapFactory
import android.graphics.Bitmap
import android.provider.OpenableColumns
import android.util.Base64
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import app.tauri.plugin.JSObject
import java.io.ByteArrayOutputStream
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
  fun readImage(invoke: Invoke) {
    // Snapshot on the UI thread while the app owns the paste gesture. Stream I/O
    // belongs off-thread and never treats an unknown provider length as huge.
    val uri = try {
      manager.primaryClip?.takeIf { it.itemCount == 1 }?.getItemAt(0)?.uri
    } catch (ex: Exception) {
      invoke.reject(ex.message ?: "Failed to read clipboard image")
      return
    }
    if (uri == null || uri.scheme != "content") {
      invoke.resolve(JSObject().apply { put("bytesB64", org.json.JSONObject.NULL) })
      return
    }
    Thread {
      try {
        val resolver = activity.contentResolver
        if (resolver.getType(uri)?.startsWith("image/") != true) {
          invoke.resolve(JSObject().apply { put("bytesB64", org.json.JSONObject.NULL) })
          return@Thread
        }
        val reportedSize = runCatching {
          resolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            val index = cursor.getColumnIndex(OpenableColumns.SIZE)
            if (cursor.moveToFirst() && index >= 0 && !cursor.isNull(index)) cursor.getLong(index) else null
          }
        }.getOrNull()
        if (reportedSize != null && reportedSize > MAX_CLIP_BYTES) {
          throw IllegalArgumentException("Image exceeds the 64 MiB limit")
        }
        val bytes = resolver.openInputStream(uri)?.use { input ->
          val out = ByteArrayOutputStream()
          val chunk = ByteArray(8192)
          while (true) {
            val count = input.read(chunk)
            if (count < 0) break
            if (out.size().toLong() + count > MAX_CLIP_BYTES) {
              throw IllegalArgumentException("Image exceeds the 64 MiB limit")
            }
            out.write(chunk, 0, count)
          }
          out.toByteArray()
        }
        // readClipboardImage promises PNG bytes. Tine's own clips are already
        // PNG; foreign providers can supply JPEG or WebP, so normalize those
        // with the same pixel budget as the desktop clipboard decoder.
        val png = bytes?.takeIf { it.isNotEmpty() }?.let { source ->
          val signature = byteArrayOf(-119, 80, 78, 71, 13, 10, 26, 10)
          if (source.size >= signature.size && source.take(8).toByteArray().contentEquals(signature)) source
          else {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(source, 0, source.size, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0 ||
              bounds.outWidth.toLong() * bounds.outHeight.toLong() > 32L * 1024L * 1024L) {
              throw IllegalArgumentException("Not a supported clipboard image")
            }
            val bitmap = BitmapFactory.decodeByteArray(source, 0, source.size)
              ?: throw IllegalArgumentException("Not a supported clipboard image")
            try {
              val out = object : ByteArrayOutputStream() {
                override fun write(buffer: ByteArray, offset: Int, length: Int) {
                  if (size().toLong() + length > MAX_CLIP_BYTES) throw IllegalArgumentException("Image exceeds the 64 MiB limit")
                  super.write(buffer, offset, length)
                }
                override fun write(value: Int) {
                  if (size().toLong() >= MAX_CLIP_BYTES) throw IllegalArgumentException("Image exceeds the 64 MiB limit")
                  super.write(value)
                }
              }
              if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)) throw IllegalArgumentException("Failed to encode clipboard image")
              out.toByteArray()
            } finally { bitmap.recycle() }
          }
        }
        val encoded = png?.let { Base64.encodeToString(it, Base64.NO_WRAP) }
        invoke.resolve(JSObject().apply { put("bytesB64", encoded ?: org.json.JSONObject.NULL) })
      } catch (ex: Exception) {
        invoke.reject(ex.message ?: "Failed to read clipboard image")
      }
    }.start()
  }

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
