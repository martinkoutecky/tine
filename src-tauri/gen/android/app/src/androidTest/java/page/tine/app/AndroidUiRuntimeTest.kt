package page.tine.app

import android.content.Context
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import android.webkit.WebView
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.abs
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Physical-input receipts for the three Android UI reports in UI-02.
 *
 * The test only uses JavaScript to observe the packaged WebView (and, for the
 * responsive matrix, set the production Android root-zoom property). Every
 * WebView control is activated through screen-level [MotionEvent]s injected by
 * Android UiAutomation; it never manufactures PointerEvent or MouseEvent objects.
 * The runner clears app data before each method, so the tap on the real
 * first-run "Create a new graph" control always produces the same demo graph.
 */
@RunWith(AndroidJUnit4::class)
class AndroidUiRuntimeTest {
  @Test
  fun responsiveChromeFitsPortraitAndLandscapeAtDefault90And110Percent() {
    withFreshDemoGraph("responsiveChromeFitsPortraitAndLandscapeAtDefault90And110Percent") { scenario, initialWebView ->
      val receipts = JSONArray()
      val fitFailures = mutableListOf<String>()
      var webView = initialWebView

      for ((orientationName, orientation) in listOf(
        "portrait" to ActivityInfo.SCREEN_ORIENTATION_PORTRAIT,
        "landscape" to ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE,
      )) {
        // The hosted emulator starts in portrait. Re-requesting the orientation
        // that is already active can recreate Tauri's surface while retaining a
        // briefly queryable old WebView. Only rotate when the requested state
        // differs, then require the replacement surface and viewport to agree.
        if (currentOrientation(scenario) != orientationName) setOrientation(scenario, orientation)
        webView = awaitWebView(scenario)
        awaitTopbar(webView)
        awaitCondition("$orientationName rendered WebView") {
          val ready = evaluateJson(webView, """
            (() => JSON.stringify({
              route: document.querySelector('.page-title')?.textContent?.trim() || '',
              blocks: document.querySelectorAll('.ls-block').length,
              width: window.innerWidth,
              height: window.innerHeight,
            }))()
          """.trimIndent())
          ready.optString("route") == "Welcome to Tine" && ready.optInt("blocks") >= 3 &&
            (if (orientationName == "landscape") ready.optDouble("width") > ready.optDouble("height")
            else ready.optDouble("height") >= ready.optDouble("width"))
        }

        for (scale in listOf(1.0, 0.9, 1.1)) {
          // Android production zoom is document-root CSS zoom (src/zoom.ts).
          // This deliberately changes the same rendered scale as the production
          // per-machine preference rather
          // than using a test-only component or desktop-width substitute.
          setRootZoom(webView, scale)
          val receipt = measureChrome(webView)
          val nativeViewport = nativeViewportState(webView)
          receipt.put("journey", "205-responsive-chrome")
          receipt.put("requestedOrientation", orientationName)
          receipt.put("orientation", currentOrientation(scenario))
          receipt.put("requestedRootZoom", scale)
          receipt.put("nativeViewport", nativeViewport)
          receipts.put(receipt)

          val clipped = receipt.optJSONArray("clipped") ?: JSONArray()
          val freeWidth = receipt.optDouble("freeWidth", Double.NEGATIVE_INFINITY)
          val containerWidth = receipt.optDouble("containerWidth", Double.NaN)
          val navigationCount = receipt.optJSONArray("directNavigation")?.length() ?: -1
          val optionalCount = receipt.optJSONArray("directOptional")?.length() ?: -1
          val sidebarDirect = receipt.optBoolean("directRightSidebar")
          val overflowVisible = receipt.optBoolean("overflowVisible")
          val winControls = receipt.optBoolean("winControls")
          val viewport = receipt.optJSONObject("viewport")
          val viewportWidth = viewport?.optDouble("innerWidth", Double.NaN) ?: Double.NaN
          val viewportHeight = viewport?.optDouble("innerHeight", Double.NaN) ?: Double.NaN
          if (clipped.length() != 0 || freeWidth < -1.0) {
            fitFailures += "$orientationName scale=$scale clipped=$clipped freeWidth=$freeWidth"
          }
          val viewportMatchesOrientation = if (orientationName == "landscape") {
            viewportWidth > viewportHeight
          } else {
            viewportHeight >= viewportWidth
          }
          if (!viewportMatchesOrientation) {
            fitFailures += "$orientationName scale=$scale viewport=${viewportWidth}x$viewportHeight did not match Android orientation"
          }
          if (winControls) {
            fitFailures += "$orientationName scale=$scale Android unexpectedly rendered desktop window controls"
          }
          if (nativeViewport.optInt("webViewTop") < nativeViewport.optInt("statusBarTop")) {
            fitFailures += "$orientationName scale=$scale WebView top ${nativeViewport.optInt("webViewTop")} remained under status inset ${nativeViewport.optInt("statusBarTop")}"
          }
          if (!containerWidth.isFinite()) {
            fitFailures += "$orientationName scale=$scale did not report a container width"
          } else {
            val expectOptional = if (containerWidth <= OPTIONAL_ACTION_FLOOR_PX) 0 else 3
            val expectNavigation = if (containerWidth <= NAVIGATION_ACTION_FLOOR_PX) 0 else 2
            val expectOverflow = containerWidth <= OPTIONAL_ACTION_FLOOR_PX
            val expectSidebarDirect = containerWidth > RIGHT_SIDEBAR_FLOOR_PX
            if (optionalCount != expectOptional || navigationCount != expectNavigation ||
              sidebarDirect != expectSidebarDirect || overflowVisible != expectOverflow) {
              fitFailures += "$orientationName scale=$scale width=$containerWidth expected optional=$expectOptional navigation=$expectNavigation sidebar=$expectSidebarDirect overflow=$expectOverflow, observed optional=$optionalCount navigation=$navigationCount sidebar=$sidebarDirect overflow=$overflowVisible"
            }
          }

          if (overflowVisible) {
            val overflow = requireNotNull(elementRectOrNull(webView, "[data-topbar-overflow-trigger]")) {
              "responsive measurement reported a visible overflow trigger without a tappable trigger"
            }
            tap(webView, overflow)
            val overflowOpened = waitForCondition {
              measureChrome(webView).optBoolean("overflowMenuVisible")
            }
            receipt.put("overflowOpenedAfterNativeTap", overflowOpened)
            if (!overflowOpened) {
              fitFailures += "$orientationName scale=$scale overflow did not open after native tap"
              continue
            }
            val opened = measureChrome(webView)
            val openedActions = jsonStrings(opened.optJSONArray("visibleOverflowActions"))
            val expectedActions = mutableListOf("calendar", "journals", "theme")
            if (containerWidth <= RIGHT_SIDEBAR_FLOOR_PX) expectedActions += "right-sidebar"
            if (containerWidth <= NAVIGATION_ACTION_FLOOR_PX) expectedActions += listOf("back", "forward")
            receipt.put("openedOverflowActions", JSONArray(openedActions))
            if (openedActions != expectedActions) {
              fitFailures += "$orientationName scale=$scale width=$containerWidth expected overflow=$expectedActions observed=$openedActions"
            }
            // Close through the same real control so the next matrix sample
            // cannot inherit an incidental overlay.
            tap(webView, requireNotNull(elementRectOrNull(webView, "[data-topbar-overflow-trigger]")))
            awaitCondition("overflow menu closed after real tap") { !measureChrome(webView).optBoolean("overflowMenuVisible") }
          }
        }
      }

      val receipt = JSONObject()
        .put("journey", "205-responsive-chrome")
        .put("samples", receipts)
        .put("fitFailures", JSONArray(fitFailures))
      emitReceipt("responsiveChromeFitsPortraitAndLandscapeAtDefault90And110Percent", receipt)
      assertTrue(
        "topbar must fit its declared direct/overflow layout without clipping: $fitFailures",
        fitFailures.isEmpty(),
      )
    }
  }

  @Test
  fun longPressPageReferenceOpensExactlyOnePageActionsMenuWithoutPreviewSelectionOrNavigation() {
    withFreshDemoGraph("longPressPageReferenceOpensExactlyOnePageActionsMenuWithoutPreviewSelectionOrNavigation") { _, webView ->
      val pageRef = awaitVisibleElementByScrolling(webView, "a.page-ref", "a rendered page reference in the demo graph")
      installLongPressMutationTrace(webView)
      val before = locationAndSelection(webView)
      longPress(webView, pageRef)
      val menuObserved = waitForCondition {
        pageActionsState(webView).optInt("pageActionsMenus") == 1
      }

      val after = pageActionsState(webView)
      val trace = readLongPressMutationTrace(webView)
      after.put("journey", "207-exclusive-page-reference-long-press")
      after.put("before", before)
      after.put("pageActionsObservedDuringWait", menuObserved)
      after.put("trace", trace)
      emitReceipt(
        "longPressPageReferenceOpensExactlyOnePageActionsMenuWithoutPreviewSelectionOrNavigation",
        after,
      )

      assertTrue("one Page actions menu must be observed after the native long press", menuObserved)
      assertEquals("one Page actions menu must remain visible", 1, after.optInt("pageActionsMenus"))
      assertEquals("long press must not open a preview", 0, after.optInt("previews"))
      assertEquals("long press must not create a text selection", "", after.optString("selection"))
      assertEquals("the menu must be inserted exactly once", 1, trace.optInt("menuAdds"))
      assertEquals("the menu must not flash closed", 0, trace.optInt("menuRemoves"))
      assertEquals("a preview must never be inserted", 0, trace.optInt("previewAdds"))
      assertEquals("a preview must never be removed after flashing", 0, trace.optInt("previewRemoves"))
      assertTrue(
        "no selectionchange sample may contain text: ${trace.optJSONArray("selectionEvents")}",
        jsonStrings(trace.optJSONArray("selectionEvents")).all(String::isEmpty),
      )
      assertTrue(
        "the active Tine route must never change: ${trace.optJSONArray("routeEvents")}",
        jsonStrings(trace.optJSONArray("routeEvents")).all { it == before.optString("route") },
      )
      assertEquals("long press must not change the active Tine route", before.optString("route"), after.optString("route"))
      assertEquals("long press must not add browser history", before.optInt("historyLength"), after.optInt("historyLength"))
    }
  }

  @Test
  fun initialNativeSelectionShowsMobileToolbarForFirstLineCaretSecondLineHold() {
    runInitialNativeSelectionJourney(
      test = "initialNativeSelectionShowsMobileToolbarForFirstLineCaretSecondLineHold",
      kind = "first-line-caret-second-line-hold",
      minimumTextLength = 140,
      maximumTextLength = Int.MAX_VALUE,
      requireSingleVisualLine = false,
    )
  }

  @Test
  fun initialNativeSelectionShowsMobileToolbarForSingleLineHold() {
    runInitialNativeSelectionJourney(
      test = "initialNativeSelectionShowsMobileToolbarForSingleLineHold",
      kind = "single-line",
      minimumTextLength = 1,
      maximumTextLength = 80,
      requireSingleVisualLine = true,
    )
  }

  private fun runInitialNativeSelectionJourney(
    test: String,
    kind: String,
    minimumTextLength: Int,
    maximumTextLength: Int,
    requireSingleVisualLine: Boolean,
  ) {
    withFreshDemoGraph(test) { scenario, webView ->
      val failures = mutableListOf<String>()
      val occurrence = "ui375-${SystemClock.elapsedRealtime()}"
      val target = awaitContentBlock(
        webView,
        minimumTextLength,
        maximumTextLength,
        requireSingleVisualLine,
        occurrence,
      )
      val blockId = target.getString("blockId")
      tapContentEditorEntry(webView, target)
      val textarea = awaitEditor(webView, blockId, occurrence, !requireSingleVisualLine)
      if (!requireSingleVisualLine) {
        // Establish the reporter's literal starting state through touch: the
        // caret is on visual line one while the keyboard is already open,
        // then the only long press lands on visual line two.
        tapAtEditorLine(webView, textarea, 0)
      }
      showIme(scenario, webView)
      val imeBefore = imeVisible(webView)
      val orientationBefore = currentOrientation(scenario)
      val before = selectionState(webView, blockId, occurrence)
      if (requireSingleVisualLine) longPress(webView, textarea) else longPressAtEditorLine(webView, textarea, 1)
      val completeStateObserved = waitForCondition(SELECTION_TIMEOUT_MS) {
        val state = selectionState(webView, blockId, occurrence)
        state.optInt("selectionLength") > 0 && state.optBoolean("toolbarVisible") &&
          state.optInt("visibleActionCount") == EXPECTED_SELECTION_ACTIONS &&
          !state.optBoolean("moreVisible") && imeVisible(webView)
      }
      val observed = selectionState(webView, blockId, occurrence)
      observed.put("kind", kind)
      observed.put("target", target)
      observed.put("caretProbeVisualLine", if (requireSingleVisualLine) JSONObject.NULL else 0)
      observed.put("holdProbeVisualLine", if (requireSingleVisualLine) 0 else 1)
      observed.put("before", before)
      observed.put("completeStateObservedDuringWait", completeStateObserved)
      observed.put("imeVisible", imeVisible(webView))
      observed.put("imeVisibleBeforeLongPress", imeBefore)
      observed.put("orientationBeforeLongPress", orientationBefore)
      observed.put("orientation", currentOrientation(scenario))

      if (!completeStateObserved || observed.optInt("selectionLength") <= 0 || !observed.optBoolean("toolbarVisible")) {
        failures += "$kind selection=${observed.optInt("selectionLength")} toolbarVisible=${observed.optBoolean("toolbarVisible")}"
      }
      if (observed.optInt("visibleActionCount") != EXPECTED_SELECTION_ACTIONS || observed.optBoolean("moreVisible")) {
        failures += "$kind formatting actions were still collapsed: ${observed.optJSONArray("visibleActionIds")}"
      }
      if (!imeBefore || !observed.optBoolean("imeVisible")) failures += "$kind IME was dismissed or never visible"
      if (!before.optBoolean("activeEditor") || before.optInt("selectionLength") != 0) {
        failures += "$kind did not begin from the intended active editor with a collapsed caret"
      }
      if (orientationBefore != observed.optString("orientation")) failures += "$kind changed orientation during selection"
      if (!requireSingleVisualLine && observed.optInt("editorVisualLines") < 2) {
        failures += "$kind did not begin on a visually wrapped editor line"
      }

      val receipt = JSONObject()
        .put("journey", "375-initial-native-selection")
        .put("selection", observed)
        .put("failures", JSONArray(failures))
      emitReceipt(test, receipt)
      assertTrue("initial native selections must retain IME and show Tine's mobile toolbar: $failures", failures.isEmpty())
    }
  }

  private fun withFreshDemoGraph(
    test: String,
    block: (ActivityScenario<MainActivity>, WebView) -> Unit,
  ) {
    val scenario = ActivityScenario.launch(MainActivity::class.java)
    var webView: WebView? = null
    try {
      val activeWebView = awaitWebView(scenario)
      webView = activeWebView
      val createDemo = awaitElementRect(activeWebView, ".welcome-choice-primary")
      // First-run onboarding itself is a user action in the packaged app.
      tap(activeWebView, createDemo)
      awaitCondition("first-run demo graph route after Create a new graph") {
        evaluateJson(activeWebView, """
          (() => JSON.stringify({
            welcomeGone: !document.querySelector('.welcome-overlay'),
            route: document.querySelector('.page-title')?.textContent?.trim() || '',
            blocks: document.querySelectorAll('.ls-block').length,
          }))()
        """.trimIndent()).let {
          it.optBoolean("welcomeGone") && it.optString("route").isNotEmpty() && it.optInt("blocks") > 0
        }
      }
      awaitTopbar(activeWebView)
      openDemoWelcomePage(activeWebView)
      block(scenario, activeWebView)
    } catch (failure: Throwable) {
      emitFailureEvidence(test, webView, failure)
      throw failure
    }
    // Do not close ActivityScenario here. On the hosted API-35 x86_64 image,
    // destroying Tauri's active WebView from the instrumentation thread aborts
    // HWUI on a destroyed mutex. The shell runner gives every method its own
    // process lifetime and force-stops the package before the next method.
  }

  private fun awaitWebView(scenario: ActivityScenario<MainActivity>): WebView {
    var found: WebView? = null
    awaitCondition("Tauri WebView") {
      scenario.onActivity { activity ->
        found = findWebView(activity.window.decorView)?.takeIf {
          it.isAttachedToWindow && it.isShown && it.width > 0 && it.height > 0
        }
      }
      found != null
    }
    return checkNotNull(found)
  }

  private fun findWebView(view: View): WebView? {
    if (view is WebView) return view
    if (view is ViewGroup) {
      for (index in 0 until view.childCount) {
        findWebView(view.getChildAt(index))?.let { return it }
      }
    }
    return null
  }

  private fun awaitTopbar(webView: WebView) {
    awaitCondition("loaded Tine topbar") { elementRectOrNull(webView, "header.topbar") != null }
  }

  /**
   * Creating the demo graph intentionally lands on today's journal. At phone
   * width the persisted-default left sidebar is also a modal drawer, so leaving
   * it open both hides the fixture and intercepts later topbar gestures. Reach
   * the demo's real Welcome favorite through native input; active navigation
   * then closes the compact drawer through the production callback.
   */
  private fun openDemoWelcomePage(webView: WebView) {
    val welcome = awaitElementRectByText(
      webView,
      "#sidebar-favorites-list .nav-page",
      "Welcome to Tine",
    )
    tap(webView, welcome)
    awaitCondition("Welcome to Tine demo page with dismissed navigation drawer") {
      evaluateJson(webView, """
        (() => JSON.stringify({
          route: document.querySelector('.page-title')?.textContent?.trim() || '',
          blocks: document.querySelectorAll('.ls-block').length,
          activeDrawer: document.querySelector('.app-container')?.getAttribute('data-active-drawer') || '',
        }))()
      """.trimIndent()).let {
        it.optString("route") == "Welcome to Tine" &&
          it.optInt("blocks") >= 3 &&
          it.optString("activeDrawer").isEmpty()
      }
    }
  }

  private fun awaitEditor(
    webView: WebView,
    blockId: String,
    occurrence: String,
    requireMultipleVisualLines: Boolean = false,
  ): JSONObject {
    var result: JSONObject? = null
    awaitCondition("real focused block textarea") {
      result = evaluateJsonOrNull(webView, """
        (() => {
          const editor = document.activeElement;
          if (!(editor instanceof HTMLTextAreaElement) || !editor.classList.contains('block-editor')) return null;
          const row = editor.closest('.ls-block');
          if (row?.dataset.blockId !== ${JSONObject.quote(blockId)} ||
              row?.dataset.androidUiOccurrence !== ${JSONObject.quote(occurrence)}) return null;
          const rect = editor.getBoundingClientRect();
          const lineHeight = parseFloat(getComputedStyle(editor).lineHeight) || 20;
          return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
            viewportWidth: window.innerWidth, viewportHeight: window.innerHeight,
            lineHeight, blockId: row.dataset.blockId, occurrence: row.dataset.androidUiOccurrence,
            active: document.activeElement === editor,
            editorVisualLines: Math.max(1, Math.round(rect.height / lineHeight)) });
        })()
      """.trimIndent())
      result != null && result!!.optBoolean("active") && (!requireMultipleVisualLines || result!!.optInt("editorVisualLines") >= 2)
    }
    return checkNotNull(result)
  }

  private fun awaitElementRect(webView: WebView, selector: String): JSONObject {
    var result: JSONObject? = null
    awaitCondition("element $selector") {
      result = elementRectOrNull(webView, selector)
      result != null
    }
    return checkNotNull(result)
  }

  private fun awaitElementRectByText(webView: WebView, selector: String, text: String): JSONObject {
    var result: JSONObject? = null
    awaitCondition("element $selector containing $text") {
      result = evaluateJsonOrNull(webView, """
        (() => {
          const element = [...document.querySelectorAll(${JSONObject.quote(selector)})]
            .find((candidate) => candidate.textContent?.includes(${JSONObject.quote(text)}));
          if (!element) return null;
          const rect = element.getBoundingClientRect();
          const style = getComputedStyle(element);
          if (style.display === 'none' || style.visibility === 'hidden' || rect.width <= 0 || rect.height <= 0) return null;
          return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
            viewportWidth: window.innerWidth, viewportHeight: window.innerHeight, dpr: window.devicePixelRatio });
        })()
      """.trimIndent())
      result != null
    }
    return checkNotNull(result)
  }

  private fun awaitVisibleElementByScrolling(webView: WebView, selector: String, description: String): JSONObject {
    repeat(MAX_FIXTURE_SCROLLS + 1) { attempt ->
      visibleElementRectOrNull(webView, selector)?.let { return it }
      if (attempt < MAX_FIXTURE_SCROLLS) swipeUp(webView)
    }
    throw AssertionError("timed out waiting for $description after $MAX_FIXTURE_SCROLLS native scroll gestures")
  }

  private fun elementRectOrNull(webView: WebView, selector: String): JSONObject? {
    val expression = """
      (() => {
        const element = document.querySelector(${JSONObject.quote(selector)});
        if (!element) return null;
        const rect = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        if (style.display === 'none' || style.visibility === 'hidden' || rect.width <= 0 || rect.height <= 0) return null;
        return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
          viewportWidth: window.innerWidth, viewportHeight: window.innerHeight, dpr: window.devicePixelRatio });
      })()
    """.trimIndent()
    return evaluateJsonOrNull(webView, expression)
  }

  private fun visibleElementRectOrNull(webView: WebView, selector: String): JSONObject? {
    val expression = """
      (() => {
        const elements = [...document.querySelectorAll(${JSONObject.quote(selector)})];
        const element = elements.find((candidate) => {
          const rect = candidate.getBoundingClientRect();
          const style = getComputedStyle(candidate);
          const safeBottom = window.innerHeight * 0.58;
          return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0 &&
            rect.top > 64 && rect.bottom < safeBottom;
        });
        if (!element) return null;
        const rect = element.getBoundingClientRect();
        return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
          viewportWidth: window.innerWidth, viewportHeight: window.innerHeight, dpr: window.devicePixelRatio });
      })()
    """.trimIndent()
    return evaluateJsonOrNull(webView, expression)
  }

  private fun awaitContentBlock(
    webView: WebView,
    minimumTextLength: Int,
    maximumTextLength: Int,
    requireSingleVisualLine: Boolean,
    occurrence: String,
  ): JSONObject {
    repeat(MAX_FIXTURE_SCROLLS + 1) { attempt ->
      val result = evaluateJsonOrNull(webView, """
        (() => {
          const candidates = [...document.querySelectorAll('.ls-block')];
          let chosen = null;
          for (const candidate of candidates) {
              const content = candidate.querySelector(':scope > .block-main > .block-content-wrapper > .block-content');
              if (!content) continue;
              const length = content.innerText.trim().length;
              const rect = content.getBoundingClientRect();
              const lineHeight = parseFloat(getComputedStyle(content).lineHeight) || 20;
              const singleLine = rect.height <= lineHeight * 1.6;
              if (!(length >= $minimumTextLength && length <= $maximumTextLength &&
                  (${if (requireSingleVisualLine) "singleLine" else "!singleLine"}) &&
                  rect.top > 64 && rect.bottom < window.innerHeight * 0.58)) continue;
              const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
              for (let node = walker.nextNode(); node; node = walker.nextNode()) {
                if (!node.textContent?.trim()) continue;
                const owner = node.parentElement;
                if (!owner || owner.closest('a,button,input,textarea,select,[contenteditable="true"]')) continue;
                const range = document.createRange();
                range.selectNodeContents(node);
                for (const textRect of Array.from(range.getClientRects())) {
                  if (textRect.width <= 2 || textRect.height <= 2) continue;
                  const x = Math.min(textRect.right - 1, textRect.left + Math.min(18, textRect.width / 2));
                  const y = textRect.top + textRect.height / 2;
                  const hit = document.elementFromPoint(x, y);
                  if (hit?.closest('.ls-block') !== candidate || hit.closest('a,button,input,textarea,select,[contenteditable="true"]')) continue;
                  chosen = { block: candidate, content, rect, lineHeight, length, x, y,
                    hitTag: hit.tagName, hitClass: String(hit.className || '') };
                  break;
                }
                if (chosen) break;
              }
              if (chosen) break;
          }
          if (!chosen) return null;
          const { block, rect, lineHeight, length, x, y, hitTag, hitClass } = chosen;
          block.dataset.androidUiOccurrence = ${JSONObject.quote(occurrence)};
          if (rect.width <= 0 || rect.height <= 0) return null;
          return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
            viewportWidth: window.innerWidth, viewportHeight: window.innerHeight,
            lineHeight, blockId: block.dataset.blockId, occurrence: ${JSONObject.quote(occurrence)},
            duplicateCount: candidates.filter((row) => row.dataset.blockId === block.dataset.blockId).length,
            textLength: length, entryX: x, entryY: y, hitTag, hitClass });
        })()
      """.trimIndent())
      if (result != null) return result
      if (attempt < MAX_FIXTURE_SCROLLS) swipeUp(webView)
    }
    throw AssertionError(
      "timed out waiting for a demo block with $minimumTextLength..$maximumTextLength visible characters " +
        "after $MAX_FIXTURE_SCROLLS native scroll gestures",
    )
  }

  private fun setRootZoom(webView: WebView, scale: Double) {
    val measured = evaluateJson(webView, """
      (() => {
        document.documentElement.style.zoom = '${scale}';
        return JSON.stringify({ rootZoom: getComputedStyle(document.documentElement).zoom || '1' });
      })()
    """.trimIndent())
    val observed = measured.optString("rootZoom").toDoubleOrNull()
    assertTrue(
      "Android root zoom must use the requested production scale: expected=$scale observed=${measured.optString("rootZoom")}",
      observed != null && abs(observed - scale) < 0.001,
    )
    SystemClock.sleep(180)
  }

  private fun tap(webView: WebView, rect: JSONObject) {
    val point = motionPoint(webView, rect)
    val downTime = SystemClock.uptimeMillis()
    dispatchMotion(webView, downTime, MotionEvent.ACTION_DOWN, point.first, point.second)
    SystemClock.sleep(TAP_SETTLE_MS)
    dispatchMotion(webView, downTime, MotionEvent.ACTION_UP, point.first, point.second)
    SystemClock.sleep(TAP_SETTLE_MS)
  }

  private fun tapContentEditorEntry(webView: WebView, content: JSONObject) {
    // The DOM probe retained a text-node Range point and proved elementFromPoint
    // resolves to this exact non-interactive occurrence. Input remains a real
    // screen-level MotionEvent; JavaScript only prevents selector guesswork.
    val cssX = content.getDouble("entryX")
    val cssY = content.getDouble("entryY")
    tapAt(webView, motionPoint(webView, content, cssX, cssY))
  }

  private fun longPress(webView: WebView, rect: JSONObject) {
    longPressAt(webView, motionPoint(webView, rect))
  }

  private fun tapAtEditorLine(webView: WebView, editor: JSONObject, line: Int) {
    val point = editorLinePoint(webView, editor, line)
    tapAt(webView, point)
  }

  private fun longPressAtEditorLine(webView: WebView, editor: JSONObject, line: Int) {
    val point = editorLinePoint(webView, editor, line)
    longPressAt(webView, point)
  }

  private fun editorLinePoint(webView: WebView, editor: JSONObject, line: Int): Pair<Float, Float> {
    val lineHeight = editor.getDouble("lineHeight")
    val cssX = editor.getDouble("left") + minOf(56.0, editor.getDouble("width") * 0.32)
    val cssY = editor.getDouble("top") + lineHeight * (line + 0.5)
    require(cssY < editor.getDouble("top") + editor.getDouble("height")) {
      "requested visual line $line lies outside the active editor: $editor"
    }
    return motionPoint(webView, editor, cssX, cssY)
  }

  private fun tapAt(webView: WebView, point: Pair<Float, Float>) {
    val downTime = SystemClock.uptimeMillis()
    dispatchMotion(webView, downTime, MotionEvent.ACTION_DOWN, point.first, point.second)
    SystemClock.sleep(TAP_SETTLE_MS)
    dispatchMotion(webView, downTime, MotionEvent.ACTION_UP, point.first, point.second)
    SystemClock.sleep(TAP_SETTLE_MS)
  }

  private fun longPressAt(webView: WebView, point: Pair<Float, Float>) {
    val downTime = SystemClock.uptimeMillis()
    dispatchMotion(webView, downTime, MotionEvent.ACTION_DOWN, point.first, point.second)
    // This is the platform long-press timeout plus a small delivery margin. The
    // action is still one DOWN/UP user gesture, not a JavaScript event sequence.
    SystemClock.sleep((ViewConfiguration.getLongPressTimeout() + LONG_PRESS_MARGIN_MS).toLong())
    dispatchMotion(webView, downTime, MotionEvent.ACTION_UP, point.first, point.second)
    SystemClock.sleep(LONG_PRESS_SETTLE_MS)
  }

  private fun swipeUp(webView: WebView) {
    val x = webView.width * 0.72f
    // The first-run demo displays two bottom toasts. Start above them so the
    // physical gesture reaches the outline scroller rather than a toast layer.
    val startY = webView.height * 0.55f
    val endY = webView.height * 0.18f
    val downTime = SystemClock.uptimeMillis()
    dispatchMotion(webView, downTime, MotionEvent.ACTION_DOWN, x, startY)
    repeat(SWIPE_STEPS) { index ->
      val fraction = (index + 1).toFloat() / SWIPE_STEPS
      val y = startY + (endY - startY) * fraction
      SystemClock.sleep(SWIPE_STEP_MS)
      dispatchMotion(webView, downTime, MotionEvent.ACTION_MOVE, x, y)
    }
    dispatchMotion(webView, downTime, MotionEvent.ACTION_UP, x, endY)
    SystemClock.sleep(SWIPE_SETTLE_MS)
  }

  private fun motionPoint(webView: WebView, rect: JSONObject): Pair<Float, Float> {
    return motionPoint(
      webView,
      rect,
      rect.getDouble("left") + rect.getDouble("width") / 2,
      rect.getDouble("top") + rect.getDouble("height") / 2,
    )
  }

  private fun motionPoint(webView: WebView, rect: JSONObject, cssX: Double, cssY: Double): Pair<Float, Float> {
    val viewportWidth = rect.getDouble("viewportWidth")
    val viewportHeight = rect.getDouble("viewportHeight")
    val x = (cssX * webView.width / viewportWidth)
      .coerceIn(1.0, (webView.width - 1).toDouble())
    val y = (cssY * webView.height / viewportHeight)
      .coerceIn(1.0, (webView.height - 1).toDouble())
    return x.toFloat() to y.toFloat()
  }

  private fun dispatchMotion(webView: WebView, downTime: Long, action: Int, x: Float, y: Float) {
    val located = CountDownLatch(1)
    val location = IntArray(2)
    webView.post {
      webView.getLocationOnScreen(location)
      located.countDown()
    }
    assertTrue("packaged WebView screen location was unavailable", located.await(EVALUATE_TIMEOUT_MS, TimeUnit.MILLISECONDS))
    val event = MotionEvent.obtain(
      downTime,
      SystemClock.uptimeMillis(),
      action,
      location[0] + x,
      location[1] + y,
      0,
    ).apply {
      source = InputDevice.SOURCE_TOUCHSCREEN
    }
    try {
      assertTrue(
        "Android UiAutomation did not inject the native MotionEvent",
        InstrumentationRegistry.getInstrumentation().uiAutomation.injectInputEvent(event, true),
      )
    } finally {
      event.recycle()
    }
  }

  private fun nativeViewportState(webView: WebView): JSONObject {
    val observed = AtomicReference<JSONObject>()
    val latch = CountDownLatch(1)
    webView.post {
      val location = IntArray(2)
      webView.getLocationOnScreen(location)
      val rootInsets = webView.rootWindowInsets
      val status = rootInsets?.getInsets(WindowInsets.Type.statusBars())
      val navigation = rootInsets?.getInsets(WindowInsets.Type.navigationBars())
      observed.set(JSONObject()
        .put("webViewLeft", location[0])
        .put("webViewTop", location[1])
        .put("webViewWidth", webView.width)
        .put("webViewHeight", webView.height)
        .put("statusBarTop", status?.top ?: 0)
        .put("navigationBarBottom", navigation?.bottom ?: 0))
      latch.countDown()
    }
    assertTrue("native WebView geometry was unavailable", latch.await(EVALUATE_TIMEOUT_MS, TimeUnit.MILLISECONDS))
    return checkNotNull(observed.get())
  }

  private fun installLongPressMutationTrace(webView: WebView) {
    val installed = evaluateJson(webView, """
      (() => {
        const state = {
          menuAdds: 0, menuRemoves: 0, previewAdds: 0, previewRemoves: 0,
          selectionEvents: [], routeEvents: [], mutations: [],
        };
        const route = () => document.querySelector('.page-title')?.textContent?.trim() || '';
        const snapshot = () => ({
          pageActions: document.querySelectorAll('[role="menu"][aria-label="Page actions"]').length,
          previews: document.querySelectorAll('.peek-popup').length,
          selection: String(getSelection()?.toString() || ''),
          route: route(), historyLength: history.length,
        });
        const matchingCount = (node, selector) => {
          if (!(node instanceof Element)) return 0;
          return (node.matches(selector) ? 1 : 0) + node.querySelectorAll(selector).length;
        };
        const observer = new MutationObserver((records) => {
          for (const record of records) {
            for (const node of record.addedNodes) {
              state.menuAdds += matchingCount(node, '[role="menu"][aria-label="Page actions"]');
              state.previewAdds += matchingCount(node, '.peek-popup');
            }
            for (const node of record.removedNodes) {
              state.menuRemoves += matchingCount(node, '[role="menu"][aria-label="Page actions"]');
              state.previewRemoves += matchingCount(node, '.peek-popup');
            }
          }
          const observed = snapshot();
          state.mutations.push(observed);
          state.routeEvents.push(observed.route);
        });
        observer.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
        document.addEventListener('selectionchange', () => state.selectionEvents.push(String(getSelection()?.toString() || '')));
        state.routeEvents.push(route());
        window.__tineAndroidUiLongPressTrace = { state, snapshot };
        return JSON.stringify({ installed: true, initial: snapshot() });
      })()
    """.trimIndent())
    assertTrue("page-reference mutation trace did not install", installed.optBoolean("installed"))
  }

  private fun readLongPressMutationTrace(webView: WebView): JSONObject {
    return evaluateJson(webView, """
      (() => JSON.stringify(window.__tineAndroidUiLongPressTrace?.state || {}))()
    """.trimIndent())
  }

  private fun locationAndSelection(webView: WebView): JSONObject = evaluateJson(webView, """
    (() => JSON.stringify({
      route: document.querySelector('.page-title')?.textContent?.trim() || '',
      historyLength: history.length,
      selection: String(getSelection()?.toString() || ''),
    }))()
  """.trimIndent())

  private fun pageActionsState(webView: WebView): JSONObject = evaluateJson(webView, """
    (() => JSON.stringify({
      pageActionsMenus: document.querySelectorAll('[role="menu"][aria-label="Page actions"]').length,
      previews: document.querySelectorAll('.peek-popup').length,
      selection: String(getSelection()?.toString() || ''),
      route: document.querySelector('.page-title')?.textContent?.trim() || '',
      historyLength: history.length,
    }))()
  """.trimIndent())

  private fun selectionState(webView: WebView, blockId: String, occurrence: String): JSONObject = evaluateJson(webView, """
    (() => {
      const editor = document.activeElement instanceof HTMLTextAreaElement &&
        document.activeElement.classList.contains('block-editor') ? document.activeElement : null;
      const row = editor?.closest('.ls-block');
      const ownsOccurrence = row?.dataset.blockId === ${JSONObject.quote(blockId)} &&
        row?.dataset.androidUiOccurrence === ${JSONObject.quote(occurrence)};
      const toolbar = document.querySelector('[data-mobile-selection-toolbar]');
      const toolbarStyle = toolbar ? getComputedStyle(toolbar) : null;
      const toolbarRect = toolbar?.getBoundingClientRect();
      return JSON.stringify({
        blockId: row?.dataset.blockId || '', occurrence: row?.dataset.androidUiOccurrence || '',
        activeEditor: !!editor && ownsOccurrence,
        selectionStart: editor?.selectionStart ?? 0,
        selectionEnd: editor?.selectionEnd ?? 0,
        selectionLength: editor ? Math.max(0, editor.selectionEnd - editor.selectionStart) : 0,
        selectedText: editor ? editor.value.slice(editor.selectionStart, editor.selectionEnd) : '',
        editorVisualLines: editor ? Math.max(1, Math.round(editor.getBoundingClientRect().height /
          (parseFloat(getComputedStyle(editor).lineHeight) || 20))) : 0,
        mobileToolbar: !!toolbar,
        toolbarVisible: !!toolbar && toolbarStyle.display !== 'none' && toolbarStyle.visibility !== 'hidden' &&
          toolbarRect.width > 0 && toolbarRect.height > 0,
        visibleActionIds: toolbar ? [...toolbar.querySelectorAll('[data-selection-action]')].filter((button) => {
          const style = getComputedStyle(button); const rect = button.getBoundingClientRect();
          return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
        }).map((button) => button.getAttribute('data-selection-action')) : [],
        visibleActionCount: toolbar ? [...toolbar.querySelectorAll('[data-selection-action]')].filter((button) => {
          const style = getComputedStyle(button); const rect = button.getBoundingClientRect();
          return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
        }).length : 0,
        moreVisible: (() => { const more = toolbar?.querySelector('.sel-toolbar-more'); if (!more) return false;
          const style = getComputedStyle(more); const rect = more.getBoundingClientRect();
          return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0; })(),
      });
    })()
  """.trimIndent())

  private fun measureChrome(webView: WebView): JSONObject = evaluateJson(webView, """
    (() => {
      const visible = (element) => {
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
      };
      const topbar = document.querySelector('header.topbar');
      if (!topbar) return null;
      const bar = topbar.getBoundingClientRect();
      const topbarStyle = getComputedStyle(topbar);
      // Popover menu buttons are descendants of the topbar component but are
      // not occupants of its direct row. Including them made the free-space
      // measurement circular whenever the menu was open.
      const direct = [...topbar.querySelectorAll('button')]
        .filter((button) => !button.closest('.topbar-overflow-menu') && visible(button));
      const navigation = [...topbar.querySelectorAll('.topbar-navigation-action')].filter(visible);
      const optional = [...topbar.querySelectorAll('.topbar-optional-action')].filter(visible);
      const sidebar = [...topbar.querySelectorAll('.topbar-sidebar-action')].filter(visible);
      const overflowTrigger = topbar.querySelector('[data-topbar-overflow-trigger]');
      const overflowMenu = topbar.querySelector('.topbar-overflow-menu');
      const clipped = direct
        .filter((button) => { const rect = button.getBoundingClientRect(); return rect.left < bar.left - 1 || rect.right > bar.right + 1; })
        .map((button) => button.getAttribute('aria-label') || button.title || button.textContent?.trim() || 'unlabelled');
      const rightEdge = direct.reduce((edge, button) => Math.max(edge, button.getBoundingClientRect().right), bar.left);
      return JSON.stringify({
        viewport: {
          innerWidth: window.innerWidth,
          innerHeight: window.innerHeight,
          visualWidth: visualViewport?.width ?? null,
          visualHeight: visualViewport?.height ?? null,
          dpr: window.devicePixelRatio,
        },
        rootZoom: getComputedStyle(document.documentElement).zoom || '1',
        // Container queries use the content box. `width` is border-box here
        // because of the global box-sizing rule, so subtract the real padding.
        containerWidth: topbar.clientWidth - parseFloat(topbarStyle.paddingLeft) - parseFloat(topbarStyle.paddingRight),
        topbar: { left: bar.left, right: bar.right, width: bar.width, height: bar.height },
        winControls: !!topbar.querySelector('.win-controls'),
        overflowVisible: !!overflowTrigger && visible(overflowTrigger),
        overflowMenuVisible: !!overflowMenu && visible(overflowMenu),
        directNavigation: navigation.map((button) => button.getAttribute('aria-label') || button.title || ''),
        directOptional: optional.map((button) => button.getAttribute('aria-label') || button.title || ''),
        directRightSidebar: sidebar.length === 1,
        directActions: direct.map((button) => {
          const rect = button.getBoundingClientRect();
          return {
            label: button.getAttribute('aria-label') || button.title || button.textContent?.trim() || 'unlabelled',
            left: rect.left, top: rect.top, width: rect.width, height: rect.height,
          };
        }),
        visibleOverflowActions: [...document.querySelectorAll('[data-topbar-overflow-action]')].filter(visible).map((element) => element.getAttribute('data-topbar-overflow-action')),
        clipped,
        freeWidth: bar.right - rightEdge,
      });
    })()
  """.trimIndent())

  private fun showIme(scenario: ActivityScenario<MainActivity>, webView: WebView) {
    scenario.onActivity { activity ->
      val input = activity.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
      webView.requestFocus()
      input.showSoftInput(webView, InputMethodManager.SHOW_IMPLICIT)
    }
    awaitCondition("Android IME visible") { imeVisible(webView) }
  }

  private fun imeVisible(webView: WebView): Boolean = webView.rootWindowInsets
    ?.isVisible(WindowInsets.Type.ime())
    ?: false

  private fun currentOrientation(scenario: ActivityScenario<MainActivity>): String {
    var orientation = "unknown"
    scenario.onActivity { activity ->
      orientation = if (activity.resources.configuration.orientation == android.content.res.Configuration.ORIENTATION_LANDSCAPE) {
        "landscape"
      } else {
        "portrait"
      }
    }
    return orientation
  }

  private fun setOrientation(scenario: ActivityScenario<MainActivity>, requested: Int) {
    scenario.onActivity { it.requestedOrientation = requested }
    val expected = if (requested == ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE) "landscape" else "portrait"
    awaitCondition("$expected Android orientation") { currentOrientation(scenario) == expected }
  }

  private fun evaluateJson(webView: WebView, expression: String): JSONObject {
    return evaluateJsonOrNull(webView, expression)
      ?: throw AssertionError("WebView observation returned no JSON for: $expression")
  }

  private fun evaluateJsonOrNull(webView: WebView, expression: String): JSONObject? {
    val latch = CountDownLatch(1)
    val raw = AtomicReference<String?>(null)
    webView.post {
      webView.evaluateJavascript(expression) { value ->
        raw.set(value)
        latch.countDown()
      }
    }
    assertTrue("timed out observing packaged WebView DOM", latch.await(EVALUATE_TIMEOUT_MS, TimeUnit.MILLISECONDS))
    val callbackValue = raw.get() ?: return null
    val decoded = JSONTokener(callbackValue).nextValue()
    if (decoded == JSONObject.NULL || decoded == null) return null
    val json = decoded as? String ?: return null
    if (json == "null") return null
    return JSONObject(json)
  }

  private fun jsonStrings(array: JSONArray?): List<String> {
    if (array == null) return emptyList()
    return (0 until array.length()).map { array.optString(it) }
  }

  private fun awaitCondition(name: String, condition: () -> Boolean) {
    if (waitForCondition(condition)) return
    throw AssertionError("timed out waiting for $name")
  }

  private fun waitForCondition(condition: () -> Boolean): Boolean {
    return waitForCondition(STARTUP_TIMEOUT_MS, condition)
  }

  private fun waitForCondition(timeoutMs: Long, condition: () -> Boolean): Boolean {
    val deadline = SystemClock.elapsedRealtime() + timeoutMs
    while (SystemClock.elapsedRealtime() < deadline) {
      if (condition()) return true
      SystemClock.sleep(POLL_MS)
    }
    return false
  }

  private fun emitReceipt(test: String, receipt: JSONObject) {
    receipt.put("test", test)
    receipt.put("device", JSONObject()
      .put("manufacturer", Build.MANUFACTURER)
      .put("model", Build.MODEL)
      .put("fingerprint", Build.FINGERPRINT)
      .put("api", Build.VERSION.SDK_INT)
      .put("webView", WebView.getCurrentWebViewPackage()?.packageName ?: "unavailable")
      .put("webViewVersion", WebView.getCurrentWebViewPackage()?.versionName ?: "unavailable"))
    val line = "TINE_ANDROID_UI_RUNTIME_RECEIPT $receipt"
    val context = ApplicationProvider.getApplicationContext<Context>()
    val directory = File(context.filesDir, "android-ui-runtime")
    require(directory.mkdirs() || directory.isDirectory) { "could not create Android UI receipt directory $directory" }
    val screenshot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
      ?: throw AssertionError("Android UiAutomation returned no in-journey screenshot for $test")
    val screenshotFile = File(directory, "$test.png")
    screenshotFile.outputStream().use { output ->
      assertTrue("could not encode in-journey screenshot for $test", screenshot.compress(Bitmap.CompressFormat.PNG, 100, output))
    }
    assertTrue("in-journey screenshot is empty for $test", screenshotFile.length() > 0)
    receipt.put("screenshot", screenshotFile.name)
    File(directory, "$test.json").writeText(receipt.toString())
    Log.i(RECEIPT_TAG, line)
    println(line)
  }

  private fun emitFailureEvidence(test: String, webView: WebView?, failure: Throwable) {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val directory = File(context.filesDir, "android-ui-runtime")
    require(directory.mkdirs() || directory.isDirectory) { "could not create Android UI evidence directory $directory" }
    val journeyReceipt = File(directory, "$test.json")
    val outcome = if (journeyReceipt.isFile && journeyReceipt.length() > 0) "product-failure" else "harness-failure"
    val receipt = JSONObject()
      .put("test", test)
      .put("outcome", outcome)
      .put("failureClass", failure.javaClass.name)
      .put("failureMessage", failure.message ?: "")
    if (webView != null) {
      try {
        receipt.put("dom", evaluateJson(webView, """
          (() => JSON.stringify({
            route: document.querySelector('.page-title')?.textContent?.trim() || '',
            blocks: document.querySelectorAll('.ls-block').length,
            pageRefs: document.querySelectorAll('a.page-ref').length,
            editors: document.querySelectorAll('textarea.block-editor').length,
            scrollY: window.scrollY,
            viewport: { width: window.innerWidth, height: window.innerHeight },
            activeTag: document.activeElement?.tagName || '',
            activeClass: document.activeElement?.className || '',
          }))()
        """.trimIndent()))
      } catch (_: Throwable) {
        receipt.put("dom", "unavailable")
      }
    }
    InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let { screenshot ->
      File(directory, "$test.png").outputStream().use { output ->
        screenshot.compress(Bitmap.CompressFormat.PNG, 100, output)
      }
      receipt.put("screenshot", "$test.png")
    }
    val evidenceFile = if (outcome == "product-failure") File(directory, "$test.failure.json") else journeyReceipt
    evidenceFile.writeText(receipt.toString())
    Log.e(RECEIPT_TAG, "TINE_ANDROID_UI_RUNTIME_FAILURE $receipt")
  }

  private companion object {
    const val RECEIPT_TAG = "TineAndroidUi"
    const val STARTUP_TIMEOUT_MS = 45_000L
    const val EVALUATE_TIMEOUT_MS = 10_000L
    const val POLL_MS = 100L
    const val TAP_SETTLE_MS = 120L
    const val LONG_PRESS_MARGIN_MS = 250
    const val LONG_PRESS_SETTLE_MS = 450L
    const val SELECTION_TIMEOUT_MS = 8_000L
    const val MAX_FIXTURE_SCROLLS = 14
    const val SWIPE_STEPS = 8
    const val SWIPE_STEP_MS = 18L
    const val SWIPE_SETTLE_MS = 320L
    const val OPTIONAL_ACTION_FLOOR_PX = 345.0
    const val NAVIGATION_ACTION_FLOOR_PX = 300.0
    const val RIGHT_SIDEBAR_FLOOR_PX = 250.0
    const val EXPECTED_SELECTION_ACTIONS = 7
  }
}
