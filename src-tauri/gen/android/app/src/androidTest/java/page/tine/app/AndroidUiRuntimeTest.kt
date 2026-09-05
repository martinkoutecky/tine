package page.tine.app

import android.content.Context
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.drawable.ColorDrawable
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
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.charset.StandardCharsets
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
 * Physical-input receipts for Android UI reports that require a packaged app.
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
          val systemInsetsOwner = receipt.optString("systemInsetsOwner")
          val appPadding = receipt.optJSONObject("appPadding") ?: JSONObject()
          val topbar = receipt.optJSONObject("topbar") ?: JSONObject()
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
          if (abs(nativeViewport.optInt("webViewTop") - nativeViewport.optInt("statusBarTop")) > NATIVE_INSET_TOLERANCE_PX) {
            fitFailures += "$orientationName scale=$scale WebView top ${nativeViewport.optInt("webViewTop")} did not meet status inset ${nativeViewport.optInt("statusBarTop")}"
          }
          if (systemInsetsOwner != "native-viewport") {
            fitFailures += "$orientationName scale=$scale Android system inset owner was '$systemInsetsOwner'"
          }
          val cssInsetValues = listOf("top", "right", "bottom", "left").map { appPadding.optDouble(it, Double.NaN) }
          if (cssInsetValues.any { !it.isFinite() || abs(it) > CSS_INSET_TOLERANCE_PX }) {
            fitFailures += "$orientationName scale=$scale native-owned app padding was $appPadding"
          }
          val topbarTop = topbar.optDouble("top", Double.NaN)
          if (!topbarTop.isFinite() || abs(topbarTop) > CSS_INSET_TOLERANCE_PX) {
            fitFailures += "$orientationName scale=$scale topbar began at $topbarTop inside the already-inset WebView"
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
  fun initialNativeSelectionShowsMobileToolbarForSingleAndWrappedLinesWithoutHandleMovement() {
    withFreshDemoGraph("initialNativeSelectionShowsMobileToolbarForSingleAndWrappedLinesWithoutHandleMovement") { scenario, webView ->
      val selections = JSONArray()
      val failures = mutableListOf<String>()

      // The two probes intentionally use only the initial long-press sequence.
      // No subsequent move event drags a native selection handle, so a green
      // receipt proves the initial-selection boundary rather than a workaround.
      for ((kind, textBounds) in listOf(
        "first-line-caret-second-line-hold" to (140 to Int.MAX_VALUE),
        // Probe the scarce wrapped fixture before opening an editor/IME. The
        // short target is abundant near the same physical scroll position.
        "single-line" to (1 to 80),
      )) {
        val target = awaitContentBlock(webView, textBounds.first, textBounds.second, kind == "single-line")
        val blockId = target.getString("blockId")
        tapContentEditorEntry(webView, target)
        val textarea = awaitEditor(webView, blockId, kind != "single-line")
        if (kind != "single-line") {
          // Establish the reporter's literal starting state through touch: the
          // caret is on visual line one while the keyboard is already open,
          // then the only long press lands on visual line two.
          tapAtEditorLine(webView, textarea, 0)
        }
        showIme(scenario, webView)
        val imeBefore = imeVisible(webView)
        val orientationBefore = currentOrientation(scenario)
        val before = selectionState(webView, blockId)
        if (kind == "single-line") longPress(webView, textarea) else longPressAtEditorLine(webView, textarea, 1)
        val completeStateObserved = waitForCondition(SELECTION_TIMEOUT_MS) {
          val state = selectionState(webView, blockId)
          state.optInt("selectionLength") > 0 && state.optBoolean("toolbarVisible") && imeVisible(webView)
        }
        val observed = selectionState(webView, blockId)
        observed.put("kind", kind)
        observed.put("caretProbeVisualLine", if (kind == "single-line") JSONObject.NULL else 0)
        observed.put("holdProbeVisualLine", if (kind == "single-line") 0 else 1)
        observed.put("before", before)
        observed.put("completeStateObservedDuringWait", completeStateObserved)
        observed.put("imeVisible", imeVisible(webView))
        observed.put("imeVisibleBeforeLongPress", imeBefore)
        observed.put("orientationBeforeLongPress", orientationBefore)
        observed.put("orientation", currentOrientation(scenario))
        selections.put(observed)

        if (!completeStateObserved || observed.optInt("selectionLength") <= 0 || !observed.optBoolean("toolbarVisible")) {
          failures += "$kind selection=${observed.optInt("selectionLength")} toolbarVisible=${observed.optBoolean("toolbarVisible")}"
        }
        if (!imeBefore || !observed.optBoolean("imeVisible")) {
          failures += "$kind IME was dismissed or never visible"
        }
        if (!before.optBoolean("activeEditor") || before.optInt("selectionLength") != 0) {
          failures += "$kind did not begin from the intended active editor with a collapsed caret"
        }
        if (orientationBefore != observed.optString("orientation")) {
          failures += "$kind changed orientation during the initial selection"
        }
        if (kind != "single-line" && observed.optInt("editorVisualLines") < 2) {
          failures += "$kind did not begin on a visually wrapped editor line"
        }
      }

      val receipt = JSONObject()
        .put("journey", "375-initial-native-selection")
        .put("selections", selections)
        .put("failures", JSONArray(failures))
      emitReceipt(
        "initialNativeSelectionShowsMobileToolbarForSingleAndWrappedLinesWithoutHandleMovement",
        receipt,
      )
      assertTrue("initial native selections must retain IME and show Tine's mobile toolbar: $failures", failures.isEmpty())
    }
  }

  @Test
  fun generatedDirectFilesPdfRouteHonorsHardwareBackHistory() {
    withFreshDemoGraph("generatedDirectFilesPdfRouteHonorsHardwareBackHistory") { scenario, webView ->
      val fixture = installGeneratedPdfLinkFixture()
      val sourceRoute = "Welcome to Tine"
      val notesRoute = "hls__android-route"
      val stages = JSONArray()

      awaitCondition("production watcher imports the generated Direct Files link") {
        pdfRouteState(webView).optInt("sourcePdfLinks") == 1
      }
      val pdfLink = awaitVisibleElementByScrolling(
        webView,
        "a.pdf-link",
        "the generated Direct Files PDF link on $sourceRoute",
      )
      stages.put(pdfRouteState(webView).put("stage", "source").put("fixture", fixture.absolutePath))

      // The route is entered through the packaged renderer's real PDF link, not
      // through JavaScript navigation or a test-only router hook.
      tap(webView, pdfLink)
      awaitCondition("generated PDF in the one mobile route surface") {
        val state = pdfRouteState(webView)
        state.optBoolean("ready") && state.optInt("viewers") == 1 &&
          state.optInt("routePanes") == 1 && state.optInt("soloRouteSurfaces") == 1 &&
          state.optInt("pageSurfaces") == 0
      }
      val opened = pdfRouteState(webView).put("stage", "pdf-opened")
      stages.put(opened)

      val find = awaitElementRect(webView, "button[title^='Find in document']")
      tap(webView, find)
      awaitCondition("PDF Find child surface") {
        pdfRouteState(webView).optInt("findBars") == 1
      }
      stages.put(pdfRouteState(webView).put("stage", "find-opened"))

      pressHardwareBack(scenario)
      awaitCondition("Hardware Back dismisses Find before leaving the PDF") {
        val state = pdfRouteState(webView)
        state.optInt("findBars") == 0 && state.optBoolean("ready") && state.optInt("viewers") == 1
      }
      stages.put(pdfRouteState(webView).put("stage", "find-dismissed"))

      // At phone width Notes deliberately lives in the production More menu.
      // Both controls are activated with native taps.
      tap(webView, awaitElementRect(webView, "button[aria-label='More settings']"))
      val notes = awaitElementRectByText(webView, ".pdf-settings-overflow button", "Notes")
      tap(webView, notes)
      awaitCondition("PDF notes route in the same mobile history") {
        val state = pdfRouteState(webView)
        state.optString("pageTitle") == notesRoute && state.optInt("viewers") == 0
      }
      stages.put(pdfRouteState(webView).put("stage", "notes-opened"))

      pressHardwareBack(scenario)
      awaitCondition("Hardware Back returns from Notes to the exact PDF route") {
        val state = pdfRouteState(webView)
        state.optBoolean("ready") && state.optString("pdfFilename") == "android-route.pdf" &&
          state.optInt("viewers") == 1 && state.optInt("routePanes") == 1
      }
      stages.put(pdfRouteState(webView).put("stage", "notes-back-to-pdf"))

      pressHardwareBack(scenario)
      awaitCondition("Hardware Back returns from PDF to the exact source page") {
        val state = pdfRouteState(webView)
        state.optString("pageTitle") == sourceRoute && state.optInt("viewers") == 0 &&
          state.optInt("sourcePdfLinks") == 1
      }
      val returned = pdfRouteState(webView).put("stage", "pdf-back-to-source")
      stages.put(returned)

      val receipt = JSONObject()
        .put("journey", "route-owned-pdf-android-back")
        .put("storageMode", "Direct Files")
        .put("fixtureOwner", "app-private generated demo graph")
        .put("sourceRoute", sourceRoute)
        .put("notesRoute", notesRoute)
        .put("stages", stages)
      emitReceipt("generatedDirectFilesPdfRouteHonorsHardwareBackHistory", receipt)

      assertEquals("PDF must occupy exactly one route pane", 1, opened.optInt("routePanes"))
      assertEquals("solo mobile layout must expose exactly one route surface", 1, opened.optInt("soloRouteSurfaces"))
      assertEquals("the PDF route must replace, not accompany, the page surface", 0, opened.optInt("pageSurfaces"))
      assertEquals("final Hardware Back must return to the exact source page", sourceRoute, returned.optString("pageTitle"))
    }
  }

  /**
   * GH #467. The system-bar and cutout insets pad the Activity content root, so
   * the strip behind the status bar is painted by the window. Its colour and the
   * bar ICON colour must come from the same authority -- Tine's own light/dark
   * choice -- or the two disagree and the notification bar goes blank: light
   * icons are white, and so is the light strip.
   *
   * This asserts the pair, not either half, because either half alone was
   * already correct before the fix.
   */
  @Test
  fun systemBarStripAndIconsAgreeWithTinesOwnThemeNotTheDeviceNightSetting() {
    val scenario = ActivityScenario.launch(MainActivity::class.java)
    try {
      for (dark in listOf(true, false, true)) {
        scenario.onActivity { activity ->
          SystemBarAppearance.apply(activity, dark)

          val expected = ContextCompat.getColor(
            activity,
            if (dark) R.color.tine_system_bar_dark else R.color.tine_system_bar_light,
          )
          val background = activity.window.decorView.background
          assertTrue(
            "the window background behind the system bars must be a flat colour, was $background",
            background is ColorDrawable,
          )
          assertEquals(
            "strip colour for dark=$dark",
            expected,
            (background as ColorDrawable).color,
          )

          val controller =
            WindowCompat.getInsetsController(activity.window, activity.window.decorView)
          assertEquals(
            "status-bar icons for dark=$dark",
            !dark,
            controller.isAppearanceLightStatusBars,
          )
          assertEquals(
            "navigation-bar icons for dark=$dark",
            !dark,
            controller.isAppearanceLightNavigationBars,
          )
          // The failure this test exists for: white-on-white. Light icons are
          // only legible on a light strip, and vice versa.
          assertTrue(
            "dark=$dark put ${if (controller.isAppearanceLightStatusBars) "dark" else "light"} " +
              "icons on a ${if (dark) "dark" else "light"} strip",
            controller.isAppearanceLightStatusBars != dark,
          )
        }
      }
    } finally {
      scenario.close()
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

  /**
   * Add one generated PDF link to the app-owned demo graph that the first-run
   * journey just created. This stays entirely in Direct Files: instrumentation
   * shares the target app UID, writes beneath its private data directory, and
   * lets the production watcher import the ordinary Markdown edit.
   */
  private fun installGeneratedPdfLinkFixture(): File {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val graphRoot = findGeneratedDirectFilesGraph(context)
    val asset = File(graphRoot, "assets/android-route.pdf")
    asset.parentFile?.let { require(it.mkdirs() || it.isDirectory) }
    asset.writeBytes(generatedPdfBytes())
    assertTrue("generated PDF fixture must be non-empty", asset.length() > 0)

    val welcome = File(graphRoot, "pages").listFiles()
      ?.singleOrNull { file ->
        file.isFile && runCatching { file.readText().contains("# Welcome to Tine") }.getOrDefault(false)
      }
      ?: throw AssertionError("could not identify the generated Welcome to Tine Markdown file under $graphRoot")
    welcome.appendText("\n- [Android route PDF](../assets/android-route.pdf)\n")
    return asset
  }

  private fun findGeneratedDirectFilesGraph(context: Context): File {
    val roots = listOfNotNull(context.filesDir, context.noBackupFilesDir, context.dataDir)
      .distinctBy { it.absolutePath }
    val graphs = roots.flatMap { root ->
      root.walkTopDown()
        .maxDepth(MAX_GRAPH_SEARCH_DEPTH)
        .filter { candidate -> File(candidate, "logseq/config.edn").isFile && File(candidate, "pages").isDirectory }
        .toList()
    }.distinctBy { it.canonicalPath }
    return graphs.singleOrNull()
      ?: throw AssertionError("expected one app-owned Direct Files graph, found ${graphs.map(File::getAbsolutePath)}")
  }

  /** Build a valid one-page PDF without checking a repository fixture into the app. */
  private fun generatedPdfBytes(): ByteArray {
    val content = "BT /F1 18 Tf 72 720 Td (Android route PDF fixture) Tj ET\n"
    val objects = listOf(
      "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
      "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
      "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n",
      "4 0 obj\n<< /Length ${content.toByteArray(StandardCharsets.US_ASCII).size} >>\nstream\n$content" +
        "endstream\nendobj\n",
      "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    )
    val output = ByteArrayOutputStream()
    fun write(value: String) = output.write(value.toByteArray(StandardCharsets.US_ASCII))
    write("%PDF-1.4\n")
    val offsets = objects.map { body ->
      val offset = output.size()
      write(body)
      offset
    }
    val xref = output.size()
    write("xref\n0 ${objects.size + 1}\n")
    write("0000000000 65535 f \n")
    offsets.forEach { write(String.format(java.util.Locale.US, "%010d 00000 n \n", it)) }
    write("trailer\n<< /Size ${objects.size + 1} /Root 1 0 R >>\nstartxref\n$xref\n%%EOF\n")
    return output.toByteArray()
  }

  private fun pdfRouteState(webView: WebView): JSONObject = evaluateJson(webView, """
    (() => JSON.stringify({
      pageTitle: document.querySelector('.page-title')?.textContent?.trim() || '',
      viewers: document.querySelectorAll('.pdf-viewer').length,
      ready: document.querySelector('.pdf-viewer')?.getAttribute('data-pdf-ready') === 'true',
      pdfFilename: document.querySelector('.pdf-viewer')?.getAttribute('data-pdf-filename') || '',
      routePanes: document.querySelectorAll('.pdf-route-pane').length,
      soloRouteSurfaces: document.querySelectorAll('.main-content-shell > .pdf-route-pane').length,
      pageSurfaces: document.querySelectorAll('.main-content-shell > main.main-content').length,
      findBars: document.querySelectorAll('.pdf-find-bar').length,
      settingsMenus: document.querySelectorAll('.pdf-settings-menu').length,
      sourcePdfLinks: document.querySelectorAll('a.pdf-link').length,
      historyLength: history.length,
    }))()
  """.trimIndent())

  private fun pressHardwareBack(scenario: ActivityScenario<MainActivity>) {
    val gesturesBefore = SafeBackBridge.gesturesReceived
    val deliveredBefore = SafeBackBridge.dispatchesDelivered
    scenario.onActivity { it.onBackPressedDispatcher.onBackPressed() }
    awaitCondition("Android hardware Back reaches and is delivered by SafeBack") {
      SafeBackBridge.gesturesReceived > gesturesBefore && SafeBackBridge.dispatchesDelivered > deliveredBefore
    }
  }

  private fun awaitEditor(webView: WebView, blockId: String, requireMultipleVisualLines: Boolean = false): JSONObject {
    var result: JSONObject? = null
    awaitCondition("real focused block textarea") {
      result = evaluateJsonOrNull(webView, """
        (() => {
          const row = document.querySelector(`.ls-block[data-block-id=${JSONObject.quote(blockId)}]`);
          const editor = row?.querySelector('textarea.block-editor');
          if (!editor) return null;
          const rect = editor.getBoundingClientRect();
          const lineHeight = parseFloat(getComputedStyle(editor).lineHeight) || 20;
          return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
            viewportWidth: window.innerWidth, viewportHeight: window.innerHeight,
            lineHeight, blockId: row.dataset.blockId,
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
          viewportWidth: window.innerWidth, viewportHeight: window.innerHeight,
          rootZoom: parseFloat(getComputedStyle(document.documentElement).zoom) || 1,
          dpr: window.devicePixelRatio });
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
  ): JSONObject {
    repeat(MAX_FIXTURE_SCROLLS + 1) { attempt ->
      val result = evaluateJsonOrNull(webView, """
        (() => {
          const block = [...document.querySelectorAll('.ls-block')]
            .find((candidate) => {
              const content = candidate.querySelector(':scope > .block-main > .block-content-wrapper');
              if (!content) return false;
              const length = content.innerText.trim().length;
              const rect = content.getBoundingClientRect();
              const lineHeight = parseFloat(getComputedStyle(content).lineHeight) || 20;
              const singleLine = rect.height <= lineHeight * 1.6;
              return length >= $minimumTextLength && length <= $maximumTextLength &&
                (${if (requireSingleVisualLine) "singleLine" else "!singleLine"}) &&
                rect.top > 64 && rect.bottom < window.innerHeight * 0.58;
            });
          if (!block) return null;
          const content = block.querySelector(':scope > .block-main > .block-content-wrapper');
          const rect = content.getBoundingClientRect();
          const lineHeight = parseFloat(getComputedStyle(content).lineHeight) || 20;
          if (rect.width <= 0 || rect.height <= 0) return null;
          return JSON.stringify({ left: rect.left, top: rect.top, width: rect.width, height: rect.height,
            viewportWidth: window.innerWidth, viewportHeight: window.innerHeight,
            lineHeight, blockId: block.dataset.blockId, textLength: content.innerText.trim().length });
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
    // The rendered text band can contain links and chips at any horizontal
    // position. The wrapper's trailing padding owns the same production
    // mousedown-to-edit path without invoking an inline control.
    val cssX = content.getDouble("left") + content.getDouble("width") - 6.0
    val cssY = content.getDouble("top") + content.getDouble("lineHeight") * 0.5
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
    // CSS root zoom changes getBoundingClientRect's coordinate space without
    // changing window.innerWidth. Convert back to the physical viewport before
    // injecting; omission previously turned a zoomed overflow tap into Settings.
    val rootZoom = rect.optDouble("rootZoom", 1.0)
    val x = (cssX * rootZoom * webView.width / viewportWidth)
      .coerceIn(1.0, (webView.width - 1).toDouble())
    val y = (cssY * rootZoom * webView.height / viewportHeight)
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
    val pointer = MotionEvent.PointerProperties().apply {
      id = 0
      toolType = MotionEvent.TOOL_TYPE_FINGER
    }
    val coordinates = MotionEvent.PointerCoords().apply {
      this.x = location[0] + x
      this.y = location[1] + y
      pressure = if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) 0f else 1f
      size = 1f
    }
    val event = MotionEvent.obtain(
      downTime, SystemClock.uptimeMillis(), action, 1,
      arrayOf(pointer), arrayOf(coordinates), 0, 0, 1f, 1f,
      0, 0, InputDevice.SOURCE_TOUCHSCREEN, 0,
    )
    try {
      assertTrue(
        "Android UiAutomation did not inject the native MotionEvent",
        InstrumentationRegistry.getInstrumentation().uiAutomation.injectInputEvent(event, true),
      )
    } finally {
      event.recycle()
    }
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

  private fun selectionState(webView: WebView, blockId: String): JSONObject = evaluateJson(webView, """
    (() => {
      const row = document.querySelector(`.ls-block[data-block-id=${JSONObject.quote(blockId)}]`);
      const editor = row?.querySelector('textarea.block-editor');
      const toolbar = document.querySelector('[data-mobile-selection-toolbar]');
      const toolbarStyle = toolbar ? getComputedStyle(toolbar) : null;
      const toolbarRect = toolbar?.getBoundingClientRect();
      return JSON.stringify({
        blockId: row?.dataset.blockId || '',
        activeEditor: document.activeElement === editor,
        selectionStart: editor?.selectionStart ?? 0,
        selectionEnd: editor?.selectionEnd ?? 0,
        selectionLength: editor ? Math.max(0, editor.selectionEnd - editor.selectionStart) : 0,
        selectedText: editor ? editor.value.slice(editor.selectionStart, editor.selectionEnd) : '',
        editorVisualLines: editor ? Math.max(1, Math.round(editor.getBoundingClientRect().height /
          (parseFloat(getComputedStyle(editor).lineHeight) || 20))) : 0,
        mobileToolbar: !!toolbar,
        toolbarVisible: !!toolbar && toolbarStyle.display !== 'none' && toolbarStyle.visibility !== 'hidden' &&
          toolbarRect.width > 0 && toolbarRect.height > 0,
      });
    })()
  """.trimIndent())

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
      const app = document.querySelector('.app-container');
      const appStyle = app ? getComputedStyle(app) : null;
      // Popover menu buttons are descendants of the topbar component but are
      // not occupants of its direct row. Including them made the free-space
      // measurement circular whenever the menu was open.
      const direct = [...topbar.querySelectorAll('button')]
        .filter((button) => !button.closest('.topbar-overflow-menu') && visible(button));
      const navigation = [...topbar.querySelectorAll('.topbar-navigation-action')].filter(visible);
      const optional = [...topbar.querySelectorAll('.topbar-optional-action')].filter(visible);
      const sidebar = topbar.querySelector('.topbar-sidebar-action');
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
        topbar: { left: bar.left, top: bar.top, right: bar.right, width: bar.width, height: bar.height },
        systemInsetsOwner: document.documentElement.dataset.systemInsets || '',
        appPadding: appStyle ? {
          top: parseFloat(appStyle.paddingTop),
          right: parseFloat(appStyle.paddingRight),
          bottom: parseFloat(appStyle.paddingBottom),
          left: parseFloat(appStyle.paddingLeft),
        } : null,
        winControls: !!topbar.querySelector('.win-controls'),
        overflowVisible: !!overflowTrigger && visible(overflowTrigger),
        overflowMenuVisible: !!overflowMenu && visible(overflowMenu),
        directNavigation: navigation.map((button) => button.getAttribute('aria-label') || button.title || ''),
        directOptional: optional.map((button) => button.getAttribute('aria-label') || button.title || ''),
        directRightSidebar: !!sidebar && visible(sidebar),
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
    const val MAX_GRAPH_SEARCH_DEPTH = 6
    const val SWIPE_STEPS = 8
    const val SWIPE_STEP_MS = 18L
    const val SWIPE_SETTLE_MS = 320L
    const val OPTIONAL_ACTION_FLOOR_PX = 345.0
    const val NAVIGATION_ACTION_FLOOR_PX = 300.0
    const val RIGHT_SIDEBAR_FLOOR_PX = 250.0
    const val NATIVE_INSET_TOLERANCE_PX = 1
    const val CSS_INSET_TOLERANCE_PX = 0.5
  }
}
