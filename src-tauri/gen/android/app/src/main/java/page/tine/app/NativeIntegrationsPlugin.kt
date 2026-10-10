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
/** Marks an intent this process already received, so a configuration change
 * or a second `load` treats it as a redelivery (looked up, not re-sent). */
private const val HANDLED_EXTRA = "page.tine.app.SHARE_HANDLED"
/** Settled publication records are kept this long (ShareState.prune). */
private const val STATE_MAX_AGE_MS = 30L * 24 * 60 * 60 * 1000

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
 * Each intent has a fingerprint and a durable publication record
 * (`ShareState`): a redelivered intent (restored Activity, Recents) resumes a
 * pending share under the same id and is skipped only once its record proves
 * it published (finding 9). A share whose process died mid-copy and is never
 * redelivered is reported at the next start, so it is never lost silently.
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
  private val state by lazy { ShareState(File(activity.filesDir, "share-state"), files) }

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
    // A launch the system restored (process death) or relaunched from Recents
    // repeats an intent that may or may not have been published: its record
    // decides.
    val redelivered = MainActivity.restoredFromSavedState ||
      (intent != null && intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0)
    if (intent != null) receive(intent, redelivered)
    Thread { settleInterrupted() }.start()
  }

  override fun onNewIntent(intent: Intent) {
    receive(intent, false)
  }

  /** Fingerprints this process is publishing now (added on the receiving
   * thread, before the worker starts). */
  private val inFlight = mutableSetOf<String>()

  private fun toast(message: String) {
    activity.runOnUiThread {
      android.widget.Toast.makeText(activity, message, android.widget.Toast.LENGTH_LONG).show()
    }
  }

  private class Share(val text: String?, val title: String?, val streams: List<Uri>, val fingerprint: String)

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
    val fingerprint = ShareIntake.fingerprint(action, intent.type, text, title, streams.map(Uri::toString))
    return Share(text, title, streams, fingerprint)
  }

  private fun receive(intent: Intent, redeliveredLaunch: Boolean) {
    val redelivered = redeliveredLaunch ||
      try {
        intent.getBooleanExtra(HANDLED_EXTRA, false)
      } catch (_: Exception) {
        false // malformed extras: decode() refuses the share below
      }
    val share = try {
      decode(intent) ?: return
    } catch (refused: ShareRefused) {
      if (!redelivered) toast(refused.message ?: "Tine couldn't save the share.")
      return
    } catch (error: Exception) {
      // Unparcelling another app's extras can throw anything (finding 10).
      Log.e(TAG, "couldn't decode a share", error)
      if (!redelivered) toast("Tine couldn't read the share. Nothing was saved.")
      return
    }
    try {
      intent.putExtra(HANDLED_EXTRA, true)
    } catch (_: Exception) {
    }
    if (!synchronized(inFlight) { inFlight.add(share.fingerprint) }) return
    val id = try {
      if (redelivered) {
        when (val record = state.lookup(share.fingerprint)) {
          // Settled, or no record: this intent was already handled (a fresh
          // share's pending record is durable before any copying starts).
          null -> null
          else -> record.id.takeIf { record.status == ShareState.Status.PENDING }
        }
      } else {
        UUID.randomUUID().toString().also {
          state.mark(share.fingerprint, ShareState.Status.PENDING, it, System.currentTimeMillis())
        }
      }
    } catch (error: Exception) {
      Log.e(TAG, "couldn't record a share", error)
      toast("Couldn't save the shared item to Tine: ${error.message}. Nothing was saved; please share it again.")
      null
    }
    if (id == null) {
      synchronized(inFlight) { inFlight.remove(share.fingerprint) }
      return
    }
    // Copying content:// streams is I/O: never on the main thread.
    Thread { publish(share, id) }.start()
  }

  private fun publish(share: Share, id: String) {
    try {
      val writer = InboxWriter(inboxRoot(), files)
      if (!writer.published(id)) {
        val resources = share.streams.mapIndexed { index, uri ->
          val type = activity.contentResolver.getType(uri) ?: "image/*"
          if (!type.startsWith("image/")) throw ShareRefused("Tine only saves shared images. Nothing was saved.")
          val name = displayName(uri) ?: "shared-${index + 1}.${type.substringAfter('/', "png").substringBefore(';')}"
          ShareResource(name, type) {
            activity.contentResolver.openInputStream(uri) ?: throw ShareRefused("$name couldn't be read. Nothing was saved.")
          }
        }
        writer.publish(id, System.currentTimeMillis(), share.text, share.title, resources)
      }
      state.mark(share.fingerprint, ShareState.Status.PUBLISHED, id, System.currentTimeMillis())
      activity.runOnUiThread { trigger(INBOX_CHANGED, JSObject()) }
    } catch (error: Exception) {
      Log.e(TAG, "couldn't save the shared item", error)
      // The user is told; a redelivery of this intent is not retried.
      try {
        state.forget(share.fingerprint)
      } catch (_: Exception) {
      }
      toast(
        (error as? ShareRefused)?.message
          ?: "Couldn't save the shared item to Tine: ${error.message}. Nothing was saved; please share it again.",
      )
    } finally {
      synchronized(inFlight) { inFlight.remove(share.fingerprint) }
    }
  }

  /** A pending share no one is publishing was cut short (process death) and
   * its intent did not come back: say so, once, and settle its record. */
  private fun settleInterrupted() {
    try {
      // This launch's own redelivery (if any) is already in `inFlight`.
      val now = System.currentTimeMillis()
      for (record in state.pending()) {
        if (synchronized(inFlight) { record.fingerprint in inFlight }) continue
        if (InboxWriter(inboxRoot(), files).published(record.id)) {
          state.mark(record.fingerprint, ShareState.Status.PUBLISHED, record.id, now)
          activity.runOnUiThread { trigger(INBOX_CHANGED, JSObject()) }
          continue
        }
        File(inboxRoot(), ".tmp-${record.id}").deleteRecursively()
        state.mark(record.fingerprint, ShareState.Status.ABANDONED, record.id, now)
        toast("A share to Tine was interrupted before it was saved. Please share it again.")
      }
      state.prune(now, STATE_MAX_AGE_MS)
    } catch (error: Exception) {
      Log.e(TAG, "couldn't check interrupted shares", error)
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
