package page.tine.app

import android.os.SystemClock
import android.webkit.WebView
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONTokener
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Who owns the hardware Back gesture once Tauri has finished starting.
 *
 * OnBackPressedDispatcher hands a gesture to the most recently added enabled
 * callback. Tauri's core AppPlugin adds one from its constructor, which runs on
 * the Rust startup thread — after MainActivity.onCreate has returned. Whenever
 * the WebView has history, AppPlugin's no-listener branch answers Back with
 * WebView.goBack() and Tine's SafeBack owner is never consulted: on screen, a
 * route quietly changes behind an open modal and the modal stays open.
 *
 * The WebView history is what makes this observable. Without it AppPlugin
 * disables itself and re-dispatches, so Back reaches Tine's owner even when
 * Tine does not own it — which is exactly why the defect survived. A run that
 * cannot arrange that history therefore FAILS: it has not disproved anything,
 * and saying so is the whole point of this fixture.
 *
 * Startup asymmetry, established from wry 0.55.1 src/android/main_pipe.rs after
 * the first device receipt (run 32174388299) failed here: inside the single
 * main-thread CreateWebView block, `load_url` is called at line 250 and
 * `on_webview_created` — which is what reaches SafeBackPlugin.load and makes
 * the WebView handle visible to this fixture — only at line 320. The handle
 * therefore appears while the first navigation is still pending, with the
 * initial empty document live. A history entry pushed onto that document is
 * discarded when the real navigation commits and replaces it, so canGoBack()
 * stayed false forever. Waiting for the handle is not waiting for the app.
 */
@RunWith(AndroidJUnit4::class)
class SafeBackOwnershipTest {
  @Test
  fun tineOwnsBackAfterTauriStartupEvenWhenTheWebViewHasHistory() {
    val scenario = ActivityScenario.launch(MainActivity::class.java)
    try {
      val webView = awaitWebViewHandle()
      // Not the handle: the app's own first document, committed.
      val document = awaitAppDocument(scenario)

      // Exactly what the production mobile router does on every navigation
      // (pushMobileHistoryEntry, src/router.ts). Deliberately not a loadUrl:
      // Tine is an SPA and never performs a second real navigation, so proving
      // ownership behind a loadUrl would prove it under conditions that never
      // occur on Martin's phone.
      val loadState = awaitLoadComplete(scenario)
      val historyBefore = evaluateInt(scenario, "history.length")
      var historyAfter = historyBefore
      var pushes = 0
      // A pushState issued while the document is still loading grows the JS
      // history without adding a WebView back/forward entry: run 32180708057
      // reported `history.length 1 -> 2` beside `canGoBack=false`, with the
      // list still holding only [initial, first commit]. Waiting for the load
      // to finish is the fix; repeating the push is the belt, because a
      // software-GL emulator can report `complete` while the first frame is
      // still settling.
      while (pushes < MAX_HISTORY_PUSHES) {
        evaluate(scenario, "history.pushState(null, '', location.href)")
        historyAfter = evaluateInt(scenario, "history.length")
        pushes += 1
        if (awaitTrue(PUSH_SETTLE_MS) { onUiThread(scenario) { webView.canGoBack() } }) break
      }

      // Last resort, and reported as such: a real same-document navigation.
      // pushState is what production does, so it is tried first and three
      // times; but a run that observes ownership under a fragment navigation
      // still answers the question this fixture exists to answer, and a run
      // that observes nothing answers nothing at all.
      var historyMode = "pushState"
      if (!onUiThread(scenario) { webView.canGoBack() }) {
        historyMode = "fragment"
        scenario.onActivity { webView.loadUrl(webView.url + "#tine-back-probe") }
        awaitTrue(SETTLE_TIMEOUT_MS) { onUiThread(scenario) { webView.canGoBack() } }
      }

      if (!onUiThread(scenario) { webView.canGoBack() }) {
        // Name the failure instead of leaving the next receipt to guess. If the
        // JS history grew but the WebView's back/forward list did not, then
        // pushState does not feed canGoBack() on this device and the production
        // premise behind pushMobileHistoryEntry is itself wrong — a different
        // finding, not a flaky fixture.
        val list = onUiThread(scenario) { webView.copyBackForwardList() }
        fail(
          "the fixture could not give the WebView a history entry, so this run " +
            "cannot observe Back ownership at all. document=$document " +
            "loadState=$loadState pushes=$pushes mode=$historyMode " +
            "history.length $historyBefore -> $historyAfter, " +
            "backForwardList size=${list.size} currentIndex=${list.currentIndex}, " +
            "canGoBack=${onUiThread(scenario) { webView.canGoBack() }}",
        )
      }

      val before = SafeBackBridge.gesturesReceived
      val deliveredBefore = SafeBackBridge.dispatchesDelivered
      scenario.onActivity { it.onBackPressedDispatcher.onBackPressed() }

      assertTrue(
        "Back did not reach Tine's SafeBack owner (history arranged by " +
          "$historyMode): another OnBackPressedCallback (Tauri's AppPlugin) was " +
          "registered later and answered the gesture with WebView history " +
          "navigation instead",
        awaitTrue(SETTLE_TIMEOUT_MS) { SafeBackBridge.gesturesReceived > before },
      )

      // Owning Back and DELIVERING it are different things, and this fixture
      // used to prove only the first. An inlined plugin with no ACL manifest
      // has `plugin:safe-back|registerListener` refused before it reaches
      // Android, so `hasListener` stays false, every gesture is consumed with
      // nowhere to send it, and the counter above still climbs. That shipped.
      assertTrue(
        "Back reached Tine's owner but was never handed to the frontend: the " +
          "\"android-safe-back\" listener is not registered, which is what an " +
          "ACL refusal of plugin:safe-back|registerListener looks like from here",
        awaitTrue(SETTLE_TIMEOUT_MS) { SafeBackBridge.dispatchesDelivered > deliveredBefore },
      )
    } finally {
      scenario.close()
    }
  }

  /** The plugin's WebView reference. Present long before the app's document is. */
  private fun awaitWebViewHandle(): WebView {
    val deadline = SystemClock.elapsedRealtime() + STARTUP_TIMEOUT_MS
    while (SystemClock.elapsedRealtime() < deadline) {
      SafeBackBridge.webViewForTest()?.let { return it }
      SystemClock.sleep(POLL_MS)
    }
    throw AssertionError("Tauri never loaded SafeBackPlugin's WebView")
  }

  /**
   * Wait for the app's own first document to COMMIT, and for nothing more than
   * that. Commit is the precondition a pushed history entry needs in order to
   * survive: Chromium replaces the initial empty document's navigation entry
   * when the real navigation commits, which is precisely what discarded the
   * entry in run 32174388299.
   *
   * A committed document is already observable from `location.href` alone, so
   * neither `readyState` nor the mounted Solid shell is required — both are
   * only reported. Demanding either would make this fixture red for reasons
   * that have nothing to do with Back ownership: `readyState` can sit at
   * `interactive` indefinitely if one subresource is slow on a software-GL
   * emulator, and a graph-less first boot shows Welcome rather than the topbar.
   * Under-constraining here is deliberate; the fail-closed guard is the
   * canGoBack() check below, which is the condition that actually matters.
   *
   * Returns the observed state, verbatim, for failure messages.
   */
  private fun awaitAppDocument(scenario: ActivityScenario<MainActivity>): String {
    var last = "(never evaluated)"
    val deadline = SystemClock.elapsedRealtime() + STARTUP_TIMEOUT_MS
    while (SystemClock.elapsedRealtime() < deadline) {
      val raw = evaluate(
        scenario,
        "JSON.stringify([location.href, document.readyState, " +
          "!!document.querySelector('header.topbar, .welcome-card, .startup-recovery-card')])",
      )
      last = raw
      val parsed = parseJsonStringArray(raw)
      if (parsed != null) {
        val href = parsed.getString(0)
        if (href.isNotEmpty() && !href.startsWith("about:")) return raw
      }
      SystemClock.sleep(POLL_MS)
    }
    throw AssertionError(
      "the app's first document never committed, so no history entry could be " +
        "pushed onto it and Back ownership was not observed. last=$last",
    )
  }

  /**
   * Wait for `document.readyState === "complete"`, and REPORT what was reached
   * rather than requiring it. A pushed history entry only becomes a WebView
   * back/forward entry once the navigation that owns it has finished loading,
   * so this wait is what makes the fixture able to observe anything at all —
   * but a slow subresource on a software-GL emulator must not be the reason a
   * Back-ownership run goes red, which is why it returns instead of failing.
   */
  private fun awaitLoadComplete(scenario: ActivityScenario<MainActivity>): String {
    var last = "(never evaluated)"
    val deadline = SystemClock.elapsedRealtime() + LOAD_COMPLETE_TIMEOUT_MS
    while (SystemClock.elapsedRealtime() < deadline) {
      last = evaluate(scenario, "document.readyState").trim('"')
      if (last == "complete") return last
      SystemClock.sleep(POLL_MS)
    }
    return last
  }

  /** `evaluateJavascript` hands back a JSON-encoded value, so a JSON.stringify
   * result arrives double-encoded: a JSON string whose contents are the array. */
  private fun parseJsonStringArray(raw: String): JSONArray? = runCatching {
    JSONArray(JSONTokener(raw).nextValue() as String)
  }.getOrNull()

  private fun evaluateInt(scenario: ActivityScenario<MainActivity>, script: String): Int =
    evaluate(scenario, script).trim('"').toIntOrNull() ?: -1

  /** Blocking evaluateJavascript. Returns the raw JSON-encoded result. */
  private fun evaluate(scenario: ActivityScenario<MainActivity>, script: String): String {
    val result = AtomicReference("null")
    val evaluated = CountDownLatch(1)
    scenario.onActivity {
      val webView = SafeBackBridge.webViewForTest()
      if (webView == null) {
        evaluated.countDown()
      } else {
        webView.evaluateJavascript(script) { value ->
          result.set(value ?: "null")
          evaluated.countDown()
        }
      }
    }
    evaluated.await(EVALUATE_TIMEOUT_MS, TimeUnit.MILLISECONDS)
    return result.get()
  }

  private fun awaitTrue(timeoutMs: Long = STARTUP_TIMEOUT_MS, condition: () -> Boolean): Boolean {
    val deadline = SystemClock.elapsedRealtime() + timeoutMs
    while (SystemClock.elapsedRealtime() < deadline) {
      if (condition()) return true
      SystemClock.sleep(POLL_MS)
    }
    return false
  }

  private fun <T> onUiThread(scenario: ActivityScenario<MainActivity>, block: () -> T): T {
    val result = AtomicReference<T>()
    scenario.onActivity { result.set(block()) }
    return result.get()
  }

  private companion object {
    const val STARTUP_TIMEOUT_MS = 60_000L
    const val EVALUATE_TIMEOUT_MS = 10_000L
    /** A pushed history entry and a dispatched Back are both immediate; this is
     * slack for the main-thread hop, not a retry budget. */
    const val SETTLE_TIMEOUT_MS = 10_000L
    /** Per push attempt; the push itself is immediate, this is main-thread slack. */
    const val PUSH_SETTLE_MS = 3_000L
    const val MAX_HISTORY_PUSHES = 3
    /** Reported, never required: see awaitLoadComplete. */
    const val LOAD_COMPLETE_TIMEOUT_MS = 20_000L
    const val POLL_MS = 100L
  }
}
