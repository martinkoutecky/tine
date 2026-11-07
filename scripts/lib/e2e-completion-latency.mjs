// Optional GH #899 measurement mode of the existing alias-completion journey.
// All timestamps use the WebView clock; WebDriver polling is outside the sample.
// Two animation frames after a reply bound the first paint of the updated rows.
// Temporary User Timing probes split ordering from render in the lane receipt;
// the ordinary journey and this sampler do not require those probes to exist.
import { waitForFileText } from "../e2e-file-poll.mjs";

export async function measureCompletionLatency(browser, sourceFile) {
  await browser.execute(() => {
    const probe = window.__completionLatency = { inputs: [], requests: [] };
    document.addEventListener("input", event => {
      const target = event.target;
      if (!target.matches("textarea.block-editor, .switcher-input")) return;
      const surface = target.matches(".switcher-input") ? "switcher" : target.value.startsWith("#") ? "tag" : "page";
      const query = surface === "switcher" ? target.value : target.value.slice(surface === "tag" ? 1 : 2, target.selectionStart);
      probe.inputs.push({ surface, query, start: performance.now() });
    }, true);
    // Tauri's invoke property is immutable. Observe its fetch transport without
    // changing a command, callback, candidate pool or response payload.
    const fetch = window.fetch;
    window.fetch = function(url, options, ...rest) {
      const command = String(url).match(/\/(quick_switch|run_graph_search)$/)?.[1];
      if (!command) return fetch.call(this, url, options, ...rest);
      const args = JSON.parse(options.body);
      const input = probe.inputs.at(-1);
      const relevant = (command === "quick_switch" && args.limit > 0) || (command === "run_graph_search" && args.pageLimit > 0);
      if (!relevant || !input || input.query !== (args.query ?? args.source)) return fetch.call(this, url, options, ...rest);
      const request = { ...input, command, pageLimit: args.pageLimit ?? args.limit, blockLimit: args.blockLimit ?? 0, sent: performance.now() };
      probe.requests.push(request);
      return fetch.call(this, url, options, ...rest).then(response => {
        const json = response.json.bind(response);
        response.json = () => json().then(answer => {
        request.reply = performance.now();
        request.candidates = command === "quick_switch" ? answer : answer.hits.filter(hit => hit.entity === "page");
        requestAnimationFrame(() => requestAnimationFrame(() => {
          const selector = input.surface === "switcher" ? ".switcher-row .switcher-name" : ".autocomplete .ac-label";
          request.labels = [...document.querySelectorAll(selector)].map(node => node.textContent.trim());
          const ordering = performance.getEntriesByType("measure")
            .filter(entry => entry.startTime >= request.reply && entry.name === (input.surface === "switcher" ? "tine:switcher-order" : "tine:completion-order"));
          request.ordering = ordering.length ? ordering.reduce((total, entry) => total + entry.duration, 0) : null;
          request.paint = performance.now();
        }));
        return answer;
        });
        return response;
      });
    };
  });
  const measurements = [];
  for (const surface of ["page", "tag", "switcher"]) {
    if (surface === "switcher") {
      // Leave the editor and settle its final authored tag before comparing
      // page-search pools. Otherwise a pending autosave can legitimately leave
      // an intermediate #tin name in just one run's immutable snapshot.
      await browser.keys(["Escape"]);
      await waitForFileText(sourceFile, text => /#tine\b/.test(text));
    }
    for (let repeat = 0; repeat < Number(process.env.TINE_LATENCY_REPEATS || 3); repeat++) {
      if (surface === "switcher") {
        await browser.keys(["Control", "k"]);
        await browser.$(".switcher-input").waitForExist({ timeout: 5000 });
      } else {
        await browser.execute(() => {
          const editor = document.querySelector("textarea.block-editor");
          editor.focus(); editor.value = ""; editor.setSelectionRange(0, 0);
          editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward" }));
        });
        for (const key of surface === "page" ? "[[" : "#") await browser.keys([key]);
      }
      let query = "";
      for (const key of "tine") {
        query += key;
        const count = await browser.execute(() => window.__completionLatency.requests.length);
        await browser.keys([key]);
        try {
          await browser.waitUntil(() => browser.execute((from, q) => window.__completionLatency.requests.slice(from)
            .some(row => row.query === q && row.paint !== undefined), count, query), { timeout: 30000, interval: 20 });
        } catch (error) {
          const state = await browser.execute(() => ({ probe: window.__completionLatency,
            raw: document.querySelector("textarea.block-editor")?.value,
            labels: [...document.querySelectorAll(".ac-label")].map(node => node.textContent) }));
          throw Error(`${surface} ${query}: ${String(error)}; ${JSON.stringify(state)}`);
        }
        const sample = await browser.execute((from, q) => window.__completionLatency.requests.slice(from).find(row => row.query === q && row.paint !== undefined), count, query);
        if (!sample.labels.length) throw Error(`No painted ${surface} rows for ${query}`);
        measurements.push({ ...sample, repeat, debounce_ms: sample.sent - sample.start,
          ipc_ms: sample.reply - sample.sent, ordering_ms: sample.ordering,
          render_ms: sample.ordering === null ? null : sample.paint - sample.reply - sample.ordering,
          response_to_paint_ms: sample.paint - sample.reply, total_ms: sample.paint - sample.start });
      }
      await browser.keys(["Escape"]);
    }
  }
  return measurements;
}
