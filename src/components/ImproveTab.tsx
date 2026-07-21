// "Help improve Tine" settings panel: run lsdoc against Logseq's own parser
// (mldoc) on YOUR graph, entirely locally, and surface anonymized + re-verified
// divergences you can paste into a bug report. The heavy lifting is in
// ../devtools/lsdoc-diff/* (a faithful port of lsdoc's graph-check.mjs). mldoc is
// lazy-loaded only when you press Run, so it costs nothing at startup.
import { createSignal, Show, For, type JSX } from "solid-js";
import {
  runComparison,
  type DiffOptions,
  type DiffReport,
  type Finding,
  type ProgressEvent,
} from "../devtools/lsdoc-diff/orchestrator";
import type { BenchSummary } from "../devtools/lsdoc-diff/bench";
import { writeClipboardText } from "../clipboard";

const ISSUES_URL = "https://github.com/martinkoutecky/lsdoc/issues";

export function ImproveTab(): JSX.Element {
  const [mode, setMode] = createSignal<"diff" | "bench" | "both">("both");
  const [journals, setJournals] = createSignal(true);
  const [fast, setFast] = createSignal(false);
  const [running, setRunning] = createSignal(false);
  const [progress, setProgress] = createSignal<ProgressEvent | null>(null);
  const [report, setReport] = createSignal<DiffReport | null>(null);
  const [error, setError] = createSignal("");
  const [copied, setCopied] = createSignal("");

  const run = async () => {
    setRunning(true);
    setError("");
    setReport(null);
    setProgress(null);
    try {
      const opts: DiffOptions = { mode: mode(), includeJournals: journals(), fast: fast(), timeoutMs: 10_000 };
      setReport(await runComparison(opts, setProgress));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
      setProgress(null);
    }
  };

  const divergences = () => (report()?.findings ?? []).filter((f): f is Extract<Finding, { type: "divergence" }> => f.type === "divergence");
  const oracleArtifacts = () => (report()?.findings ?? []).filter((f): f is Extract<Finding, { type: "mldoc-oracle-artifact" }> => f.type === "mldoc-oracle-artifact");
  const otherFindings = () => (report()?.findings ?? []).filter((f) => f.type !== "divergence" && f.type !== "mldoc-oracle-artifact");

  const flash = async (text: string, key: string) => {
    try {
      await writeClipboardText(text);
      setCopied(key);
    } catch {
      setCopied(`fail:${key}`);
    }
    setTimeout(() => setCopied(""), 1600);
  };

  const findingMarkdown = (f: Extract<Finding, { type: "divergence" }>): string => {
    if (!f.anonymized.ok) return `### ${f.rel} (lines ${f.lineStart}-${f.lineEnd})\nDivergence found but not auto-anonymizable — omitted.\n`;
    const fence = pickFence(f.anonymized.input);
    return [
      `### ${f.rel} (lines ${f.lineStart}-${f.lineEnd})${f.contextDependent ? " — whole-page context" : ""}`,
      `Anonymized & re-verified (via ${f.anonymized.tier}):`,
      `${fence}`,
      f.anonymized.input,
      `${fence}`,
      `lsdoc: \`${truncate(f.anonymized.lsdocKey, 400)}\``,
      `mldoc: \`${truncate(f.anonymized.mldocKey, 400)}\``,
      "",
    ].join("\n");
  };

  const reportMarkdown = (): string => {
    const ds = divergences();
    return [
      "## lsdoc divergences from my Tine graph",
      "",
      `Tine version: ${report()?.tineVersion ?? "unknown"}`,
      `lsdoc version: ${report()?.lsdocVersion ?? "unknown"}`,
      "",
      "These snippets are anonymized (page content scrubbed) and each still reproduces the divergence between lsdoc and Logseq's mldoc.",
      "",
      ...ds.map(findingMarkdown),
      `Reported via Tine → Settings → Help improve Tine. Post to ${ISSUES_URL}`,
    ].join("\n");
  };

  return (
    <div class="improve-tab">
      <p class="settings-hint">
        Tine's parser (<b>lsdoc</b>) is a from-scratch reimplementation of the parser Logseq itself uses (
        <b>mldoc</b>). This runs both on <b>your</b> graph, entirely on this machine, and finds any place they
        disagree — which is how you can help make Tine render your notes exactly like Logseq.
      </p>
      <p class="settings-hint">
        <b>Privacy:</b> nothing is uploaded. Every divergence snippet shown below is <b>anonymized</b> (page names
        and words replaced; URL schemes kept but hosts and paths scrubbed) and <b>re-checked</b> to confirm it still
        reproduces the bug. Tine omits a finding when that privacy-preserving scrub loses the mismatch. Still read
        every snippet carefully before you choose to share it.
      </p>

      <div class="settings-field">
        <div class="settings-field-row">
          <span class="settings-label">What to run</span>
          <div class="settings-field-control">
            <div class="seg">
              <For each={["both", "diff", "bench"] as const}>
                {(m) => (
                  <button class="seg-btn" classList={{ on: mode() === m }} onClick={() => setMode(m)} disabled={running()}>
                    {m === "both" ? "Both" : m === "diff" ? "Divergences" : "Speed"}
                  </button>
                )}
              </For>
            </div>
          </div>
        </div>
      </div>

      <div class="settings-field">
        <div class="settings-field-row">
          <span class="settings-label">Include journals</span>
          <div class="settings-field-control">
            <Toggle on={journals()} onClick={() => setJournals(!journals())} disabled={running()} />
          </div>
        </div>
      </div>

      <div class="settings-field">
        <div class="settings-field-row">
          <span class="settings-label">Fast scan</span>
          <div class="settings-field-control">
            <Toggle on={fast()} onClick={() => setFast(!fast())} disabled={running()} />
          </div>
        </div>
        <div class="settings-hint settings-field-hint">
          Faster, but can miss or invent divergences; only confirmed ones are shown. Leave off for a thorough,
          authoritative scan.
        </div>
      </div>

      <div class="improve-run">
        <button class="btn-primary" onClick={run} disabled={running()}>
          {running() ? "Running…" : "Run comparison"}
        </button>
        <Show when={running() && progress()}>
          {(p) => (
            <span class="settings-hint improve-progress">
              {phaseLabel(p().phase)} {p().done}/{p().total}
              <Show when={p().current}> · {p().current}</Show>
            </span>
          )}
        </Show>
      </div>

      <Show when={error()}>
        <div class="improve-error">Failed: {error()}</div>
      </Show>

      <Show when={report()}>
        {(r) => (
          <div class="improve-report">
            <div class="settings-hint">
              Tine {r().tineVersion} · lsdoc {r().lsdocVersion} · Scanned {r().stats.files} file(s), {fmtBytes(r().stats.totalBytes)}.
            </div>

            <Show when={!r().lsdocAvailable}>
              <div class="improve-notice">
                Divergence detection needs the next lsdoc build (0.3.4) — this build can't yet parse whole files with
                lsdoc in-app. Speed numbers for Logseq's parser are shown below; the lsdoc column and divergence
                scan will light up after the update.
              </div>
            </Show>

            <Show when={r().bench}>
              {(b) => (
                <div class="improve-bench">
                  <h4>Parse speed (best of 3)</h4>
                  <table class="improve-table">
                    <thead>
                      <tr><th>Parser</th><th>Files</th><th>Total</th><th>p50</th><th>p95</th><th>max</th></tr>
                    </thead>
                    <tbody>
                      <BenchRow name="lsdoc (Tine)" s={b().lsdoc} />
                      <BenchRow name="mldoc (Logseq)" s={b().mldoc} />
                    </tbody>
                  </table>
                </div>
              )}
            </Show>

            <Show when={!!r().findings && r().lsdocAvailable}>
              <div class="improve-findings">
                <div class="improve-findings-head">
                  <h4>Divergences ({divergences().length})</h4>
                  <Show when={divergences().length > 0}>
                    <button class="btn-secondary" onClick={() => flash(reportMarkdown(), "all")}>
                      {copied() === "all" ? "Copied!" : copied() === "fail:all" ? "Copy failed" : "Copy all"}
                    </button>
                  </Show>
                </div>
                <Show when={divergences().length === 0}>
                  <div class="improve-clean">
                    No actionable divergences — lsdoc matched Logseq's parser on every file after verified mldoc oracle artifacts were quarantined.
                  </div>
                </Show>
                <For each={divergences()}>
                  {(f) => (
                    <div class="improve-card">
                      <div class="improve-card-head">
                        <code>{f.rel}</code>
                        <span class="improve-lines">lines {f.lineStart}-{f.lineEnd}</span>
                      </div>
                      <Show when={f.anonymized.ok ? f.anonymized : null} keyed fallback={<div class="settings-hint">Found, but couldn't be anonymized — not shown.</div>}>
                        {(a) => (
                          <>
                            <div class="settings-hint">anonymized via {a.tier}</div>
                            <pre class="improve-snippet">{a.input}</pre>
                            <button class="btn-secondary" onClick={() => flash(findingMarkdown(f), f.rel)}>
                              {copied() === f.rel ? "Copied!" : copied() === `fail:${f.rel}` ? "Copy failed" : "Copy"}
                            </button>
                          </>
                        )}
                      </Show>
                    </div>
                  )}
                </For>
                <Show when={oracleArtifacts().length > 0}>
                  <details class="improve-other">
                    <summary>{oracleArtifacts().length} suppressed mldoc oracle artifact(s)</summary>
                    <For each={oracleArtifacts()}>
                      {(f) => (
                        <div class="settings-hint">
                          <code>{f.rel}</code> · lines {f.lineStart}-{f.lineEnd} — {f.detail}
                        </div>
                      )}
                    </For>
                  </details>
                </Show>
                <Show when={otherFindings().length > 0}>
                  <details class="improve-other">
                    <summary>{otherFindings().length} non-divergence issue(s)</summary>
                    <For each={otherFindings()}>
                      {(f) => <div class="settings-hint"><code>{"rel" in f ? f.rel : ""}</code> — {f.type}</div>}
                    </For>
                  </details>
                </Show>
                <Show when={divergences().length > 0}>
                  <p class="settings-hint">Post these anonymized snippets to <a href={ISSUES_URL} target="_blank" rel="noreferrer">{ISSUES_URL}</a></p>
                </Show>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

function BenchRow(props: { name: string; s: BenchSummary | null }): JSX.Element {
  return (
    <tr>
      <td>{props.name}</td>
      <Show when={props.s} fallback={<td colSpan={5} class="settings-hint">pending next build</td>}>
        {(s) => (
          <>
            <td>{s().fileCount}</td>
            <td>{s().totalMs.toFixed(1)} ms</td>
            <td>{s().p50Ms.toFixed(2)}</td>
            <td>{s().p95Ms.toFixed(2)}</td>
            <td>{s().maxMs.toFixed(2)}</td>
          </>
        )}
      </Show>
    </tr>
  );
}

function Toggle(props: { on: boolean; onClick: () => void; disabled?: boolean }): JSX.Element {
  return (
    <button class="settings-toggle" classList={{ on: props.on }} role="switch" aria-checked={props.on} onClick={props.onClick} disabled={props.disabled}>
      <span class="settings-toggle-knob" />
    </button>
  );
}

function phaseLabel(p: ProgressEvent["phase"]): string {
  return p === "scan" ? "Scanning" : p === "verify" ? "Verifying" : "Timing";
}
function fmtBytes(n: number): string {
  return n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;
}
function truncate(s: string, max: number): string {
  return s.length <= max ? s : `${s.slice(0, max)}...`;
}
function pickFence(text: string): string {
  let fence = "```";
  while (text.includes(fence)) fence += "`";
  return fence;
}
