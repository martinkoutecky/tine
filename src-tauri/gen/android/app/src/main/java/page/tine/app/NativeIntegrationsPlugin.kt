package page.tine.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.system.Os
import android.system.OsConstants
import android.util.Log
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.util.UUID

private const val TAG = "Tine/NativeIntegrations"
private const val INBOX_CHANGED = "inboxChanged"
/** The occurrence id stamped on a share intent at its first receipt: the
 * inbox item's id. MainActivity carries it across process death in the saved
 * instance state (the system restores the intent without app-added extras). */
internal const val SHARE_OCCURRENCE_EXTRA = "page.tine.app.SHARE_OCCURRENCE"

/**
 * The Android producer of Tine's share inbox (ADR 0073; GH #608).
 *
 * `ACTION_SEND` / `ACTION_SEND_MULTIPLE` (text, links, images) become one inbox
 * item under `filesDir/share-inbox/` (ShareIntake.kt `InboxWriter`: written
 * into `.tmp-<id>/`, every file and the directory synced, published by one
 * rename, the inbox synced), then the frontend is told (`inboxChanged`). This
 * class never touches the graph: the app ingests the item through its single
 * journal writer (src/shareIngest.ts) and removes it only after that write
 * reached disk.
 *
 * Every share is decoded and validated on the calling thread before any work
 * starts, and is saved whole or refused with a message: only `content:`
 * images from another app's provider, at most 32 (review findings 5, 6, 10).
 * Each share occurrence gets a random id at its first receipt, stamped on the
 * intent (review round 2). Every delivery of it, fresh or repeated (restored
 * Activity, Recents, a second `load`), goes through `ShareIntake.deliver`: an
 * item or commit tombstone with that id means it was published, so the inbox
 * is synced and the arrival announced; otherwise it is (re)published. Two
 * shares with equal content are two occurrences and two items. A redelivery
 * whose files can no longer be read is refused with a message, never dropped.
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

  private val files = DurableFiles { dir ->
    val fd = Os.open(dir.absolutePath, OsConstants.O_RDONLY, 0)
    try {
      Os.fsync(fd)
    } finally {
      Os.close(fd)
    }
  }
  @Command
  fun inboxDirectory(invoke: Invoke) {
    val root = inboxRoot()
    try {
      files.ensureDir(root)
    } catch (error: Exception) {
      invoke.reject("couldn't create the share inbox: ${error.message}")
      return
    }
    invoke.resolve(JSObject().apply { put("path", root.absolutePath) })
  }

  override fun load(webView: WebView) {
    val intent = activity.intent
    // A redelivered occurrence (restored intent) is republished, not reported.
    val keep = try {
      intent?.getStringExtra(SHARE_OCCURRENCE_EXTRA)
    } catch (_: Exception) {
      null
    }
    Thread {
      try {
        for (lost in InboxWriter(inboxRoot(), files).interrupted(keep)) {
          // The marker goes only once its notice is shown (R3-2); a crash
          // before that repeats the notice at the next start.
          activity.runOnUiThread {
            android.widget.Toast.makeText(
              activity,
              "A share to Tine was interrupted and wasn't saved: ${lost.summary}. Please share it again.",
              android.widget.Toast.LENGTH_LONG,
            ).show()
            lost.acknowledge()
          }
        }
      } catch (error: Exception) {
        Log.e(TAG, "couldn't check interrupted shares", error)
      }
    }.start()
    intent?.let(::receive)
  }

  override fun onNewIntent(intent: Intent) {
    receive(intent)
  }

  private fun toast(message: String) {
    activity.runOnUiThread {
      android.widget.Toast.makeText(activity, message, android.widget.Toast.LENGTH_LONG).show()
    }
  }

  private class Share(val text: String?, val title: String?, val streams: List<Uri>)

  /** Decode and validate the whole intent; null when it is no share. */
  @Suppress("DEPRECATION")
  private fun decode(intent: Intent): Share? {
    val action = intent.action
    if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return null
    val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.takeIf { it.isNotBlank() }
    val title = intent.getCharSequenceExtra(Intent.EXTRA_SUBJECT)?.toString()
    val raw = if (intent.type?.startsWith("image/") == true) intent.extras?.get(Intent.EXTRA_STREAM) else null
    val streams = ShareIntake.streams(raw, action == Intent.ACTION_SEND_MULTIPLE) { it as? Uri }
    for (uri in streams) {
      ShareIntake.checkSource(uri.scheme, uri.authority) { authority ->
        authority.startsWith("${activity.packageName}.") ||
          activity.packageManager.resolveContentProvider(authority, 0)?.packageName == activity.packageName
      }
    }
    if (text == null && streams.isEmpty()) throw ShareRefused("The share had nothing Tine can save.")
    return Share(text, title, streams)
  }

  private fun receive(intent: Intent) {
    val share = try {
      decode(intent) ?: return
    } catch (refused: ShareRefused) {
      toast(refused.message ?: "Tine couldn't save the share.")
      return
    } catch (error: Exception) {
      // Unparcelling another app's extras can throw anything (finding 10).
      Log.e(TAG, "couldn't decode a share", error)
      toast("Tine couldn't read the share. Nothing was saved.")
      return
    }
    // Decoded, so the extras unparcel: stamp the occurrence on this thread,
    // before any copy starts.
    val id = intent.getStringExtra(SHARE_OCCURRENCE_EXTRA)?.takeIf(ShareIntake::validId)
      ?: UUID.randomUUID().toString().also { intent.putExtra(SHARE_OCCURRENCE_EXTRA, it) }
    // Copying content:// streams is I/O: never on the main thread.
    Thread { publish(share, id) }.start()
  }

  private fun publish(share: Share, id: String) {
    try {
      val writer = InboxWriter(inboxRoot(), files)
      val announce = ShareIntake.deliver(id, writer) {
        val resources = share.streams.mapIndexed { index, uri ->
          val type = activity.contentResolver.getType(uri) ?: "image/*"
          if (!type.startsWith("image/")) throw ShareRefused("Tine only saves shared images. Nothing was saved.")
          val name = displayName(uri) ?: "shared-${index + 1}.${type.substringAfter('/', "png").substringBefore(';')}"
          ShareResource(name, type) {
            val stream = try {
              activity.contentResolver.openInputStream(uri)
            } catch (_: SecurityException) {
              null // the read grant ended (a redelivery after the sender's grant expired)
            }
            stream ?: throw ShareRefused("$name couldn't be read. Nothing was saved; please share it again.")
          }
        }
        writer.publish(id, System.currentTimeMillis(), share.text, share.title, resources, ShareIntake.summary(share.text ?: share.title, resources.size))
      }
      if (announce) activity.runOnUiThread { trigger(INBOX_CHANGED, JSObject()) }
    } catch (unconfirmed: ShareUnconfirmed) {
      // Visible, so it is ingested as usual; the user is told it may not
      // have survived, and its marker stays for the next start's check.
      Log.e(TAG, "couldn't make a share durable", unconfirmed)
      toast(unconfirmed.message ?: "Tine couldn't make sure the share was saved.")
      activity.runOnUiThread { trigger(INBOX_CHANGED, JSObject()) }
    } catch (error: Exception) {
      Log.e(TAG, "couldn't save the shared item", error)
      toast(
        (error as? ShareRefused)?.message
          ?: "Couldn't save the shared item to Tine: ${error.message}. Nothing was saved; please share it again.",
      )
    }
  }

  private fun displayName(uri: Uri): String? = try {
    activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) cursor.getString(0) else null
    }
  } catch (_: Exception) {
    null
  }
}
