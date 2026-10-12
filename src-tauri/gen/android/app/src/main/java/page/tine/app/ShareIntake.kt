package page.tine.app

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream

/**
 * The platform-free half of the Android share producer (ADR 0073; GH #608):
 * what a share intent may carry, how an inbox item reaches disk, and what a
 * (re)delivery of one share occurrence does. Plain JVM code, so `app/src/test`
 * covers it without a device. `NativeIntegrationsPlugin` supplies the Android
 * parts (intent extras, the content resolver, directory fsync through `Os`).
 */

/** Must match share_inbox.rs (`MAX_RESOURCES`). */
const val MAX_SHARE_RESOURCES = 32
const val MAX_SHARE_RESOURCE_BYTES = 64L * 1024L * 1024L

/** A share Tine will not save, with the message the user sees. Every refusal
 * keeps the whole share out: nothing is saved in part (review finding 6). */
class ShareRefused(message: String) : Exception(message)

/** The item is visible in the inbox but its publishing rename could not be
 * made durable (review round 3, R3-1): it is still ingested, its
 * `.receiving-<id>` marker is kept, and the user is told. */
class ShareUnconfirmed(message: String) : Exception(message)

object ShareIntake {
  /**
   * `EXTRA_STREAM` as delivered: one Parcelable for SEND, a list for
   * SEND_MULTIPLE. Each entry must be a `Uri` (`isUri`); another Parcelable,
   * a null entry or a missing list is refused before any work starts
   * (finding 10: a malformed share from another app is in scope).
   * More than [MAX_SHARE_RESOURCES] entries are refused, never truncated.
   */
  fun <U : Any> streams(raw: Any?, multiple: Boolean, cast: (Any) -> U?): List<U> {
    if (raw == null) return emptyList()
    val entries: List<Any?> = if (multiple) {
      (raw as? List<*>) ?: throw ShareRefused("The shared images couldn't be read.")
    } else {
      listOf(raw)
    }
    if (entries.size > MAX_SHARE_RESOURCES) {
      throw ShareRefused("Tine saves at most $MAX_SHARE_RESOURCES images from one share; this one had ${entries.size}. Nothing was saved.")
    }
    return entries.map { entry ->
      entry?.let(cast) ?: throw ShareRefused("A shared item wasn't an image Tine can read. Nothing was saved.")
    }
  }

  /**
   * Only another app's content provider may supply a shared file (finding 5,
   * threat: a hostile app asks the exported share target to copy a file only
   * Tine can read). `file:` and other schemes name raw paths that Tine would
   * open with its own permissions; a provider Tine owns serves Tine's files.
   */
  fun checkSource(scheme: String?, authority: String?, ownedByTine: (String) -> Boolean) {
    if (!"content".equals(scheme, ignoreCase = true) || authority.isNullOrEmpty()) {
      throw ShareRefused("Tine only accepts images shared by another app. Nothing was saved.")
    }
    if (ownedByTine(authority)) {
      throw ShareRefused("Tine can't import its own files through sharing. Nothing was saved.")
    }
  }

  /** An occurrence id the inbox accepts as an item name (share_inbox.rs `valid_id`). */
  fun validId(id: String): Boolean =
    id.isNotEmpty() && id.length <= 64 && id.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '-' || it == '_' }

  /** Ids this process is delivering now: one copy per occurrence at a time. */
  private val delivering = mutableSetOf<String>()

  fun isDelivering(id: String): Boolean = synchronized(delivering) { id in delivering }

  /** What the user shared, for an "interrupted" notice. */
  fun summary(text: String?, images: Int): String = when {
    text != null -> if (text.length > 60) text.take(60).trimEnd() + "…" else text
    images == 1 -> "1 image"
    else -> "$images images"
  }

  /**
   * One delivery of occurrence `id` (fresh, restored, from history or seen
   * before): if its item or its commit tombstone exists, it was published, so
   * the inbox is synced (the barrier a cut-short earlier delivery may have
   * missed) and the arrival announced; otherwise `publish` writes it. False
   * when another delivery of the same occurrence is running in this process
   * (that one announces). Errors propagate.
   */
  fun deliver(id: String, writer: InboxWriter, publish: () -> Unit): Boolean {
    if (!synchronized(delivering) { delivering.add(id) }) return false
    try {
      if (writer.handled(id)) writer.syncInbox() else publish()
      return true
    } finally {
      synchronized(delivering) { delivering.remove(id) }
    }
  }

  /** A plain file name inside the item, unique among `used`. */
  fun fileName(name: String, used: MutableSet<String>): String {
    var base = name.replace('/', '_').replace('\\', '_').replace("\u0000", "").trim()
    if (base.isEmpty() || base == "." || base == ".." || base == "item.json" || base == "prepared.json") base = "file"
    if (base.length > 200) base = base.takeLast(200)
    var candidate = base
    var n = 1
    while (!used.add(candidate)) candidate = "${n++}-$base"
    return candidate
  }

  fun jsonString(text: String): String {
    val out = StringBuilder("\"")
    for (c in text) {
      when {
        c == '"' -> out.append("\\\"")
        c == '\\' -> out.append("\\\\")
        c == '\n' -> out.append("\\n")
        c == '\r' -> out.append("\\r")
        c == '\t' -> out.append("\\t")
        c < ' ' -> out.append("\\u%04x".format(c.code))
        else -> out.append(c)
      }
    }
    return out.append('"').toString()
  }
}

/** One file of a share, opened only while it is copied. */
class ShareResource(val name: String, val type: String, val open: () -> InputStream)

/**
 * Durable file operations. `syncDirectory` makes a directory's entries
 * durable (Android: `Os.fsync` on an `O_RDONLY` descriptor); errors propagate.
 */
class DurableFiles(private val syncDirectory: (File) -> Unit) {
  fun write(target: File, write: (FileOutputStream) -> Unit) {
    FileOutputStream(target).use { out ->
      write(out)
      out.fd.sync()
    }
  }

  fun syncDir(dir: File) = syncDirectory(dir)

  /** `mkdirs` that also makes `dir`'s entry durable in its parent on every
   * call: an existing directory may be left from a creation whose parent
   * sync failed (review round 3, R3-1). One fsync. */
  fun ensureDir(dir: File) {
    if (dir.isDirectory) {
      dir.parentFile?.let(syncDirectory)
      return
    }
    dir.parentFile?.takeIf { !it.isDirectory }?.let(::ensureDir)
    if (!dir.mkdir() && !dir.isDirectory) throw IOException("couldn't create ${dir.name}")
    dir.parentFile?.let(syncDirectory)
  }

  fun rename(from: File, to: File) {
    if (!from.renameTo(to)) throw IOException("couldn't rename ${from.name} to ${to.name}")
  }

  /** Replace `target` atomically and durably with `text`. */
  fun replace(target: File, text: String) {
    val tmp = File(target.parentFile, ".tmp-${target.name}")
    write(tmp) { it.write(text.toByteArray(Charsets.UTF_8)) }
    rename(tmp, target)
    syncDirectory(target.parentFile!!)
  }
}

/**
 * Writes one inbox item: everything into `.tmp-<id>/`, each file synced, the
 * directory synced, one rename to `<id>`, then the inbox synced (finding 4:
 * the item must survive power loss before its producer reports success).
 */
class InboxWriter(private val root: File, private val files: DurableFiles) {
  fun published(id: String): Boolean = File(root, id).isDirectory

  /** Published at some point: the item, or the tombstone its commit left
   * (share_inbox.rs `commit`, `.committed-<id>`, kept 30 days). */
  fun handled(id: String): Boolean = published(id) || File(root, ".committed-$id").exists()

  fun syncInbox() = files.syncDir(root)

  /** A share cut short before it was published: say `summary`, then call
   * `acknowledge` (it deletes the marker), only once the notice is shown. */
  class Interrupted(val summary: String, private val marker: File) {
    fun acknowledge() {
      marker.delete()
    }
  }

  /**
   * Leftover `.receiving-<id>` markers, except `keep`'s (being redelivered
   * now) and in-flight ones. A marker whose item or tombstone exists is
   * settled: deleted after the inbox is synced (a visible item is not yet
   * a durable one); if that sync fails, it stays for the next start. Any
   * other marker is returned, still on disk, and is deleted only by
   * [Interrupted.acknowledge] (review round 3, R3-2). Each marker is read
   * on its own; an unreadable one is reported without its summary.
   */
  fun interrupted(keep: String?): List<Interrupted> {
    val lost = ArrayList<Interrupted>()
    val settled = ArrayList<File>()
    for (name in root.list() ?: emptyArray()) {
      if (!name.startsWith(RECEIVING)) continue
      val id = name.removePrefix(RECEIVING)
      if (id == keep || ShareIntake.isDelivering(id)) continue
      val marker = File(root, name)
      if (handled(id)) {
        settled.add(marker)
        continue
      }
      val summary = try {
        marker.readText(Charsets.UTF_8)
      } catch (_: Exception) {
        "a shared item"
      }
      lost.add(Interrupted(summary, marker))
    }
    if (settled.isNotEmpty()) {
      try {
        files.syncDir(root)
        settled.forEach { it.delete() }
      } catch (_: Exception) {
        // Kept: the next start syncs and checks again.
      }
    }
    return lost
  }

  companion object {
    const val RECEIVING = ".receiving-"
  }

  /**
   * Publish occurrence `id`. A durable `.receiving-<id>` marker holding
   * `summary` exists from before the copy until the item is published (or
   * refused with a message), so a share whose process dies mid-copy is
   * reported at the next start ([ShareIntake.interrupted]).
   */
  fun publish(id: String, created: Long, text: String?, title: String?, resources: List<ShareResource>, summary: String) {
    files.ensureDir(root)
    val marker = File(root, "$RECEIVING$id")
    files.replace(marker, summary)
    try {
      write(id, created, text, title, resources)
    } catch (unconfirmed: ShareUnconfirmed) {
      throw unconfirmed // the marker stays: the item may not survive power loss
    } catch (error: Exception) {
      marker.delete() // refused or failed before publishing; the caller says so
      throw error
    }
    marker.delete()
  }

  private fun write(id: String, created: Long, text: String?, title: String?, resources: List<ShareResource>) {
    val tmp = File(root, ".tmp-$id")
    tmp.deleteRecursively() // an earlier attempt of the same share, cut short
    if (!tmp.mkdir()) throw IOException("couldn't create a share inbox item")
    try {
      val used = mutableSetOf<String>()
      val entries = ArrayList<String>()
      for (resource in resources) {
        val file = ShareIntake.fileName(resource.name, used)
        resource.open().use { source ->
          files.write(File(tmp, file)) { out ->
            val buffer = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
              val read = source.read(buffer)
              if (read < 0) break
              total += read
              if (total > MAX_SHARE_RESOURCE_BYTES) {
                throw ShareRefused("${resource.name} is larger than 64 MiB. Nothing was saved.")
              }
              out.write(buffer, 0, read)
            }
          }
        }
        entries.add("{\"file\":${ShareIntake.jsonString(file)},\"name\":${ShareIntake.jsonString(resource.name)},\"type\":${ShareIntake.jsonString(resource.type)}}")
      }
      val json = StringBuilder("{\"version\":1,\"source\":\"android\",\"created\":$created")
      if (text != null) json.append(",\"text\":").append(ShareIntake.jsonString(text))
      if (!title.isNullOrBlank()) json.append(",\"title\":").append(ShareIntake.jsonString(title))
      json.append(",\"resources\":[").append(entries.joinToString(",")).append("]}")
      files.write(File(tmp, "item.json")) { it.write(json.toString().toByteArray(Charsets.UTF_8)) }
      files.syncDir(tmp)
      files.rename(tmp, File(root, id))
    } catch (error: Exception) {
      tmp.deleteRecursively()
      throw error
    }
    // Renamed: the item is visible and will be ingested. Its rename is
    // durable only after this sync; retried once, then reported (R3-1).
    try {
      files.syncDir(root)
    } catch (_: Exception) {
      try {
        files.syncDir(root)
      } catch (error: Exception) {
        throw ShareUnconfirmed("Tine couldn't make sure the share was saved (${error.message}). If it doesn't appear in today's journal, please share it again.")
      }
    }
  }
}
