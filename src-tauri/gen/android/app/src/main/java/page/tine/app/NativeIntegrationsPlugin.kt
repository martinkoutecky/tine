package page.tine.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.util.Log
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.util.UUID

private const val TAG = "Tine/NativeIntegrations"
/** Must match share_inbox.rs (`MAX_RESOURCES`) and the frontend listener. */
private const val MAX_RESOURCES = 32
private const val MAX_RESOURCE_BYTES = 64L * 1024L * 1024L
private const val INBOX_CHANGED = "inboxChanged"
/** Marks a share intent this process already published, so a configuration
 * change or a second `load` never publishes it twice. */
private const val HANDLED_EXTRA = "page.tine.app.SHARE_HANDLED"

/**
 * The Android producer of Tine's share inbox (ADR 0073; GH #608).
 *
 * `ACTION_SEND` / `ACTION_SEND_MULTIPLE` (text, links, images) become one inbox
 * item under `filesDir/share-inbox/`: everything is written into
 * `.tmp-<id>/`, synced, and published by one rename, then the frontend is told
 * (`inboxChanged`). This class never touches the graph: the app ingests the item
 * through its single journal writer (src/shareIngest.ts) and removes it only
 * after that write reached disk.
 *
 * Mapping, after OG's Android `SendIntent` payload (`frontend/mobile/intent.cljs`
 * `handle-result`): `EXTRA_TEXT` is the item text (OG's `:url`, which
 * `transform-args` splits into highlight and link), `EXTRA_SUBJECT` the title,
 * `EXTRA_STREAM` images the resources.
 *
 * Routes (launcher shortcuts, the Quick Settings tile) are plain `tine://`
 * VIEW intents and need nothing here.
 */
@TauriPlugin
class NativeIntegrationsPlugin(private val activity: Activity) : Plugin(activity) {
  private fun inboxRoot(): File = File(activity.filesDir, "share-inbox")

  @Command
  fun inboxDirectory(invoke: Invoke) {
    val root = inboxRoot()
    if (!root.isDirectory && !root.mkdirs()) {
      invoke.reject("couldn't create the share inbox")
      return
    }
    invoke.resolve(JSObject().apply { put("path", root.absolutePath) })
  }

  override fun load(webView: WebView) {
    val intent = activity.intent ?: return
    // A launch the system restored (process death) or relaunched from Recents
    // repeats an intent that was already handled when it first arrived.
    if (MainActivity.restoredFromSavedState) return
    if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
    receive(intent)
  }

  override fun onNewIntent(intent: Intent) {
    receive(intent)
  }

  private fun receive(intent: Intent) {
    if (intent.action != Intent.ACTION_SEND && intent.action != Intent.ACTION_SEND_MULTIPLE) return
    if (intent.getBooleanExtra(HANDLED_EXTRA, false)) return
    intent.putExtra(HANDLED_EXTRA, true)
    val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
    val title = intent.getStringExtra(Intent.EXTRA_SUBJECT)
    val streams = streamsOf(intent)
    // Copying content:// streams is I/O: never on the main thread.
    Thread {
      try {
        if (publish(text, title, streams)) activity.runOnUiThread { trigger(INBOX_CHANGED, JSObject()) }
      } catch (error: Exception) {
        Log.e(TAG, "couldn't save the shared item", error)
        activity.runOnUiThread {
          android.widget.Toast.makeText(activity, "Couldn't save the shared item to Tine: ${error.message}", android.widget.Toast.LENGTH_LONG).show()
        }
      }
    }.start()
  }

  @Suppress("DEPRECATION")
  private fun streamsOf(intent: Intent): List<Uri> {
    if (intent.type?.startsWith("image/") != true) return emptyList()
    return if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
      val list = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
      else intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)
      list.orEmpty().take(MAX_RESOURCES)
    } else {
      val uri = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
      else intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
      listOfNotNull(uri)
    }
  }

  private fun displayName(uri: Uri): String? = try {
    activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) cursor.getString(0) else null
    }
  } catch (_: Exception) {
    null
  }

  /** A plain file name inside the item, unique among `used`. */
  private fun fileName(name: String, used: MutableSet<String>): String {
    var base = name.replace('/', '_').replace('\\', '_').replace("\u0000", "").trim()
    if (base.isEmpty() || base == "." || base == ".." || base == "item.json" || base == "prepared.json") base = "file"
    if (base.length > 200) base = base.takeLast(200)
    var candidate = base
    var n = 1
    while (!used.add(candidate)) candidate = "${n++}-$base"
    return candidate
  }

  private fun writeSynced(target: File, write: (FileOutputStream) -> Unit) {
    FileOutputStream(target).use { out ->
      write(out)
      out.fd.sync()
    }
  }

  /** Publish one item. Returns false when the share carried nothing usable. */
  private fun publish(text: String?, title: String?, streams: List<Uri>): Boolean {
    val cleanText = text?.takeIf { it.isNotBlank() }
    if (cleanText == null && streams.isEmpty()) return false
    val root = inboxRoot()
    if (!root.isDirectory && !root.mkdirs()) throw IllegalStateException("couldn't create the share inbox")
    val id = UUID.randomUUID().toString()
    val tmp = File(root, ".tmp-$id")
    if (!tmp.mkdir()) throw IllegalStateException("couldn't create a share inbox item")
    try {
      val used = mutableSetOf<String>()
      val resources = JSONArray()
      for ((index, uri) in streams.withIndex()) {
        val type = activity.contentResolver.getType(uri) ?: "image/*"
        val name = displayName(uri) ?: "shared-${index + 1}.${type.substringAfter('/', "png").substringBefore(';')}"
        val file = fileName(name, used)
        val input = activity.contentResolver.openInputStream(uri) ?: throw IllegalStateException("couldn't read $name")
        input.use { source ->
          writeSynced(File(tmp, file)) { out ->
            val buffer = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
              val read = source.read(buffer)
              if (read < 0) break
              total += read
              if (total > MAX_RESOURCE_BYTES) throw IllegalStateException("$name is larger than 64 MiB")
              out.write(buffer, 0, read)
            }
          }
        }
        resources.put(JSONObject().put("file", file).put("name", name).put("type", type))
      }
      val item = JSONObject()
        .put("version", 1)
        .put("created", System.currentTimeMillis())
        .put("resources", resources)
      if (cleanText != null) item.put("text", cleanText)
      if (!title.isNullOrBlank()) item.put("title", title)
      writeSynced(File(tmp, "item.json")) { it.write(item.toString().toByteArray(Charsets.UTF_8)) }
      if (!tmp.renameTo(File(root, id))) throw IllegalStateException("couldn't publish the share inbox item")
      return true
    } catch (error: Exception) {
      tmp.deleteRecursively()
      throw error
    }
  }
}
