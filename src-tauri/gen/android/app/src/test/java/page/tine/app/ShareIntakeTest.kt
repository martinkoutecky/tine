package page.tine.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.io.InputStream

/** ADR 0073 Android producer; review round 1 findings 4, 5, 6, 9, 10. */
class ShareIntakeTest {
  @get:Rule
  val temp = TemporaryFolder()

  /** Stands in for android.net.Uri: the decoder only needs a type check. */
  private data class FakeUri(val text: String)

  private fun refused(block: () -> Unit): String {
    try {
      block()
    } catch (refusal: ShareRefused) {
      return refusal.message!!
    }
    fail("expected a refusal")
    error("unreachable")
  }

  private val asUri: (Any) -> FakeUri? = { it as? FakeUri }

  // ---- Finding 10: malformed extras are refused before any work. ----

  @Test
  fun aWrongParcelableOrNullEntryIsRefusedNotCrashed() {
    assertTrue(refused { ShareIntake.streams("an Intent, not a Uri", false, asUri) }.contains("Nothing was saved"))
    assertTrue(refused { ShareIntake.streams(listOf(FakeUri("a"), null), true, asUri) }.contains("Nothing was saved"))
    assertTrue(refused { ShareIntake.streams(listOf(FakeUri("a"), 42), true, asUri) }.contains("Nothing was saved"))
    // SEND_MULTIPLE whose extra is not a list at all.
    assertTrue(refused { ShareIntake.streams(FakeUri("a"), true, asUri) }.isNotEmpty())
    assertEquals(emptyList<FakeUri>(), ShareIntake.streams(null, true, asUri))
    assertEquals(listOf(FakeUri("a")), ShareIntake.streams(FakeUri("a"), false, asUri))
  }

  // ---- Finding 6: more than the limit is refused, never truncated. ----

  @Test
  fun overTheLimitIsRefusedWhole() {
    val ok = List(MAX_SHARE_RESOURCES) { FakeUri("$it") }
    assertEquals(MAX_SHARE_RESOURCES, ShareIntake.streams(ok, true, asUri).size)
    val message = refused { ShareIntake.streams(ok + FakeUri("33"), true, asUri) }
    assertTrue(message, message.contains("33") && message.contains("Nothing was saved"))
  }

  // ---- Finding 5: only another app's content provider. ----

  @Test
  fun onlyAnotherAppsContentUriIsAccepted() {
    val ours: (String) -> Boolean = { it == "page.tine.app.fileprovider" }
    ShareIntake.checkSource("content", "com.android.providers.media.documents", ours)
    ShareIntake.checkSource("CONTENT", "com.example.gallery", ours)
    refused { ShareIntake.checkSource("file", null, ours) }
    refused { ShareIntake.checkSource("file", "", ours) }
    refused { ShareIntake.checkSource(null, "x", ours) }
    refused { ShareIntake.checkSource("android.resource", "page.tine.app", ours) }
    refused { ShareIntake.checkSource("content", null, ours) }
    val own = refused { ShareIntake.checkSource("content", "page.tine.app.fileprovider", ours) }
    assertTrue(own, own.contains("own files"))
  }

  // ---- Finding 4: durable publication, in order. ----

  private class Recorder {
    val log = mutableListOf<String>()
    var failSyncOf: String? = null
    val files = DurableFiles { dir ->
      if (dir.name == failSyncOf) throw IOException("fsync failed")
      log.add("sync ${dir.name}")
    }
  }

  private fun resource(name: String, bytes: ByteArray) = ShareResource(name, "image/png") { ByteArrayInputStream(bytes) }

  @Test
  fun anItemIsSyncedBeforeAndAfterItsPublishingRename() {
    val recorder = Recorder()
    val root = File(temp.root, "share-inbox")
    val writer = InboxWriter(root, recorder.files)
    writer.publish("id1", 7, "hello \"x\"\n", "Title", listOf(resource("a.png", byteArrayOf(1, 2, 3))))
    // The inbox is created durably, the item directory synced before the
    // rename, the inbox synced after it.
    assertEquals(listOf("sync ${temp.root.name}", "sync .tmp-id1", "sync share-inbox"), recorder.log)
    val item = File(root, "id1")
    assertTrue(item.isDirectory)
    assertFalse(File(root, ".tmp-id1").exists())
    assertEquals(3, File(item, "a.png").length())
    val json = File(item, "item.json").readText()
    assertTrue(json, json.startsWith("{\"version\":1,\"source\":\"android\",\"created\":7,"))
    assertTrue(json, json.contains("\"text\":\"hello \\\"x\\\"\\n\""))
    assertTrue(json, json.contains("\"resources\":[{\"file\":\"a.png\",\"name\":\"a.png\",\"type\":\"image/png\"}]"))
    assertTrue(writer.published("id1"))
  }

  @Test
  fun aFailedSyncIsAnErrorAndLeavesNothingPublished() {
    val recorder = Recorder()
    val root = File(temp.root, "share-inbox")
    recorder.failSyncOf = ".tmp-id1"
    try {
      InboxWriter(root, recorder.files).publish("id1", 7, "t", null, emptyList())
      fail("expected the sync error")
    } catch (_: IOException) {
    }
    assertFalse(File(root, "id1").exists())
    assertFalse(File(root, ".tmp-id1").exists())
  }

  @Test
  fun anOversizedOrUnreadableFileRefusesTheWholeItem() {
    val root = File(temp.root, "share-inbox")
    val huge = object : InputStream() {
      override fun read(): Int = 0
      override fun read(b: ByteArray, off: Int, len: Int): Int = len
    }
    val message = refused {
      InboxWriter(root, Recorder().files).publish(
        "id1", 7, "t", null,
        listOf(resource("ok.png", byteArrayOf(1)), ShareResource("big.png", "image/png") { huge }),
      )
    }
    assertTrue(message, message.contains("big.png"))
    assertFalse(File(root, "id1").exists())
    assertFalse(File(root, ".tmp-id1").exists())
    refused {
      InboxWriter(root, Recorder().files).publish(
        "id2", 7, null, null,
        listOf(ShareResource("gone.png", "image/png") { throw ShareRefused("gone.png couldn't be read. Nothing was saved.") }),
      )
    }
    assertFalse(File(root, "id2").exists())
  }

  @Test
  fun aRetryUnderTheSameIdReplacesAnAbandonedStagingDirectory() {
    val root = File(temp.root, "share-inbox").apply { mkdirs() }
    File(root, ".tmp-id1").mkdir()
    File(root, ".tmp-id1/partial.png").writeText("half")
    InboxWriter(root, Recorder().files).publish("id1", 7, "t", null, emptyList())
    assertEquals(setOf("item.json"), File(root, "id1").list()!!.toSet())
  }

  // ---- Finding 9: publication state survives the process. ----

  @Test
  fun aRecordReadsBackAcrossInstancesAndPendingOnesAreListed() {
    val dir = File(temp.root, "share-state")
    val files = Recorder().files
    ShareState(dir, files).mark("fp1", ShareState.Status.PENDING, "id1", 100)
    ShareState(dir, files).mark("fp2", ShareState.Status.PENDING, "id2", 100)
    ShareState(dir, files).mark("fp2", ShareState.Status.PUBLISHED, "id2", 200)
    val fresh = ShareState(dir, files) // a new process
    assertEquals(ShareState.Record("fp1", ShareState.Status.PENDING, "id1", 100), fresh.lookup("fp1"))
    assertEquals(ShareState.Status.PUBLISHED, fresh.lookup("fp2")!!.status)
    assertEquals(listOf("fp1"), fresh.pending().map { it.fingerprint })
    assertNull(fresh.lookup("fp3"))
    assertFalse(File(dir, ".tmp-fp1").exists())
  }

  @Test
  fun pruningDropsOnlyOldSettledRecords() {
    val dir = File(temp.root, "share-state")
    val state = ShareState(dir, Recorder().files)
    state.mark("old", ShareState.Status.PUBLISHED, "a", 0)
    state.mark("oldAbandoned", ShareState.Status.ABANDONED, "b", 0)
    state.mark("oldPending", ShareState.Status.PENDING, "c", 0)
    state.mark("new", ShareState.Status.PUBLISHED, "d", 900)
    state.prune(1000, 500)
    assertNull(state.lookup("old"))
    assertNull(state.lookup("oldAbandoned"))
    assertEquals(ShareState.Status.PENDING, state.lookup("oldPending")!!.status)
    assertEquals(ShareState.Status.PUBLISHED, state.lookup("new")!!.status)
  }

  @Test
  fun theFingerprintIsStableAndSeparatesFields() {
    val a = ShareIntake.fingerprint("SEND", "text/plain", "ab", null, emptyList())
    assertEquals(a, ShareIntake.fingerprint("SEND", "text/plain", "ab", null, emptyList()))
    assertNotEquals(a, ShareIntake.fingerprint("SEND", "text/plain", "a", "b", emptyList()))
    assertNotEquals(a, ShareIntake.fingerprint("SEND", "text/plain", "ab", "", emptyList()))
    assertNotEquals(
      ShareIntake.fingerprint("SEND_MULTIPLE", "image/*", null, null, listOf("content://x/1", "content://x/2")),
      ShareIntake.fingerprint("SEND_MULTIPLE", "image/*", null, null, listOf("content://x/2", "content://x/1")),
    )
  }

  @Test
  fun fileNamesStayInsideTheItem() {
    val used = mutableSetOf<String>()
    assertEquals(".._.._x", ShareIntake.fileName("../../x", used))
    assertEquals("file", ShareIntake.fileName("item.json", used))
    assertEquals("1-file", ShareIntake.fileName("..", used))
  }
}
