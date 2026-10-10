package page.tine.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.io.InputStream

/** ADR 0073 Android producer; review round 1 findings 4, 5, 6, 10 and round 2 R2-2/3/4. */
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

  @Test
  fun aSyncFailureAfterThePublishingRenameLeavesTheItemPublished() {
    val recorder = Recorder()
    val root = File(temp.root, "share-inbox")
    recorder.failSyncOf = "share-inbox"
    val writer = InboxWriter(root, recorder.files)
    // Not reported as unsaved: the item is visible and will be ingested.
    writer.publish("id1", 7, "t", null, emptyList())
    assertTrue(writer.published("id1"))
    assertFalse(File(root, ".tmp-id1").exists())
  }

  // ---- Review round 2 (R2-2, R2-3, R2-4): one item per share occurrence. ----

  private fun deliver(id: String, writer: InboxWriter, published: MutableList<String>) =
    ShareIntake.deliver(id, writer) {
      published.add(id)
      writer.publish(id, 7, "same text", null, emptyList())
    }

  @Test
  fun aRedeliveredOccurrenceWithNoItemAndNoTombstoneIsPublishedAgain() {
    val root = File(temp.root, "share-inbox")
    val published = mutableListOf<String>()
    // A restored intent whose first delivery died before publishing.
    assertTrue(deliver("occ-1", InboxWriter(root, Recorder().files), published))
    assertEquals(listOf("occ-1"), published)
    assertTrue(File(root, "occ-1").isDirectory)
  }

  @Test
  fun aPublishedOrCommittedOccurrenceIsAnnouncedAfterAnInboxSyncNotRepublished() {
    val root = File(temp.root, "share-inbox").apply { mkdirs() }
    File(root, "occ-1").mkdir() // renamed; its inbox sync may have been cut short
    File(root, ".committed-occ-2").writeText("") // ingested and removed
    for (id in listOf("occ-1", "occ-2")) {
      val recorder = Recorder()
      val published = mutableListOf<String>()
      assertTrue(deliver(id, InboxWriter(root, recorder.files), published))
      assertEquals(emptyList<String>(), published)
      assertEquals(listOf("sync share-inbox"), recorder.log)
    }
    assertFalse(File(root, "occ-2").exists())
  }

  @Test
  fun twoOccurrencesWithEqualContentAreTwoItems() {
    val root = File(temp.root, "share-inbox")
    val writer = InboxWriter(root, Recorder().files)
    val published = mutableListOf<String>()
    deliver("occ-1", writer, published)
    deliver("occ-2", writer, published)
    assertEquals(listOf("occ-1", "occ-2"), published)
    assertEquals(setOf("occ-1", "occ-2"), root.list()!!.filter { !it.startsWith(".") }.toSet())
  }

  @Test
  fun aConcurrentDeliveryOfTheSameOccurrenceDoesNotCopyItAgain() {
    val root = File(temp.root, "share-inbox")
    val writer = InboxWriter(root, Recorder().files)
    var inner: Boolean? = null
    val outer = ShareIntake.deliver("occ-1", writer) {
      // A second delivery of the same occurrence arrives mid-copy.
      inner = ShareIntake.deliver("occ-1", writer) { fail("copied twice") }
      writer.publish("occ-1", 7, "t", null, emptyList())
    }
    assertTrue(outer)
    assertEquals(false, inner)
    // Once the first finished, a later delivery finds the item.
    assertTrue(ShareIntake.deliver("occ-1", writer) { fail("republished") })
  }

  @Test
  fun occurrenceIdsAreValidInboxNames() {
    assertTrue(ShareIntake.validId(java.util.UUID.randomUUID().toString()))
    assertFalse(ShareIntake.validId(""))
    assertFalse(ShareIntake.validId("../x"))
    assertFalse(ShareIntake.validId("a".repeat(65)))
  }

  @Test
  fun fileNamesStayInsideTheItem() {
    val used = mutableSetOf<String>()
    assertEquals(".._.._x", ShareIntake.fileName("../../x", used))
    assertEquals("file", ShareIntake.fileName("item.json", used))
    assertEquals("1-file", ShareIntake.fileName("..", used))
  }
}
