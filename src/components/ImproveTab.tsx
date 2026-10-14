import { Rich, richValue } from "./ThemeChoice";
import { t } from "../i18n";
// "Help improve Tine" settings panel: run lsdoc against Logseq's own parser
// (mldoc) on YOUR graph, entirely locally, and surface anonymized + re-verified
// divergences you can paste into a bug report. The heavy lifting is in
// ../devtools/lsdoc-diff/* (a faithful port of lsdoc's graph-check.mjs). mldoc is
// lazy-loaded only when you press Run, so it costs nothing at startup.
import { APP_PRODUCT_NAME } from "../appIdentity";
import { createSignal, Show, For, onCleanup, type JSX } from "solid-js";
import { graphOwner, latestOwner } from "../owned";
import {
  runComparison,
  type DiffOptions,
  type DiffReport,
  type Finding,
  type ProgressEvent,
} from "../devtools/lsdoc-diff/orchestrator";
import type { BenchSummary } from "../devtools/lsdoc-diff/bench";
import { writeClipboardTextStrict } from "../clipboard";

const ISSUES_URL = "https://github.com/martinkoutecky/lsdoc/issues";

export function ImproveTab(): JSX.Element {
  const requestScope = {};
  let alive = true;
  onCleanup(() => { alive = false; });
  const [mode, setMode] = createSignal<"diff" | "bench" | "both">("both");
  const [journals, setJournals] = createSignal(true);
  const [fast, setFast] = createSignal(false);
  const [running, setRunning] = createSignal(false);
  const [progress, setProgress] = createSignal<ProgressEvent | null>(null);
  const [report, setReport] = createSignal<DiffReport | null>(null);
  const [error, setError] = createSignal("");
  const [copied, setCopied] = createSignal("");

  const run = async () => {
    const owner = latestOwner(requestScope, "comparison", graphOwner(() => alive));
    setRunning(true);
    setError("");
    setReport(null);
    setProgress(null);
    try {
      const opts: DiffOptions = { mode: mode(), includeJournals: journals(), fast: fast(), timeoutMs: 10_000 };
      const result = await runComparison(opts, (event) => { if (owner()) setProgress(event); }, owner);
      if (result.kind === "current") setReport(result.value);
    } catch (e) {
      if (owner()) setError(String(e));
    } finally {
      if (owner()) { setRunning(false); setProgress(null); }
    }
  };

  const divergences = () => (report()?.findings ?? []).filter((f): f is Extract<Finding, { type: "divergence" }> => f.type === "divergence");
  const oracleArtifacts = () => (report()?.findings ?? []).filter((f): f is Extract<Finding, { type: "mldoc-oracle-artifact" }> => f.type === "mldoc-oracle-artifact");
  const intentionalDivergences = () => (report()?.findings ?? []).filter((f): f is Extract<Finding, { type: "intentional-divergence" }> => f.type === "intentional-divergence");
  const otherFindings = () => (report()?.findings ?? []).filter((f) => f.type !== "divergence" && f.type !== "mldoc-oracle-artifact" && f.type !== "intentional-divergence");

  const flash = async (text: string, key: string) => {
    try {
      await writeClipboardTextStrict(text);
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
      `${APP_PRODUCT_NAME} version: ${report()?.tineVersion ?? "unknown"}`,
      `lsdoc version: ${report()?.lsdocVersion ?? "unknown"}`,
      "",
      "These snippets are anonymized (page content scrubbed) and each still reproduces the divergence between lsdoc and Logseq's mldoc.",
      "",
      ...ds.map(findingMarkdown),
      `Reported via Tine → Settings → Help & diagnostics. Post to ${ISSUES_URL}`,
    ].join("\n");
  };

  return (
    <div class="improve-tab">
      <p class="settings-hint"><Rich text={t("improve-tine-s-parser-lsdoc-is-a-from-scratch-reimplementation")} slots={{ "strong": (children) => <b>{children}</b>, "strong1": (children) => <b>{children}</b>, "strong2": (children) => <b>{children}</b> }} /></p>
      <p class="settings-hint"><Rich text={t("improve-privacy-nothing-is-uploaded-every-divergence-snippet")} slots={{ "strong": (children) => <b>{children}</b>, "strong1": (children) => <b>{children}</b>, "strong2": (children) => <b>{children}</b> }} /></p>

      <div class="settings-field">
        <div class="settings-field-row"><span class="settings-label">{t("improve-what-to-run")}</span>
          <div class="settings-field-control">
            <div class="seg">
              <For each={["both", "diff", "bench"] as const}>
                {(m) => (
                  <button class="seg-btn" classList={{ on: mode() === m }} onClick={() => setMode(m)} disabled={running()}>{m === "both" ? t("improve-both") : m === "diff" ? t("improve-divergences") : t("improve-speed")}
                  </button>
                )}
              </For>
            </div>
          </div>
        </div>
      </div>

      <div class="settings-field">
        <div class="settings-field-row"><span class="settings-label">{t("improve-include-journals")}</span>
          <div class="settings-field-control">
            <Toggle on={journals()} onClick={() => setJournals(!journals())} disabled={running()} />
          </div>
        </div>
      </div>

      <div class="settings-field">
        <div class="settings-field-row"><span class="settings-label">{t("improve-fast-scan")}</span>
          <div class="settings-field-control">
            <Toggle on={fast()} onClick={() => setFast(!fast())} disabled={running()} />
          </div>
        </div>
        <div class="settings-hint settings-field-hint">{t("improve-faster-but-can-miss-or-invent-divergences")}</div>
      </div>

      <div class="improve-run">
        <button class="btn-primary" onClick={run} disabled={running()}>{running() ? t("improve-running") : t("improve-run-comparison")}
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

      <Show when={error()}><div class="improve-error">{t("improve-failed", { error: error() })}</div>
      </Show>

      <Show when={report()}>
        {(r) => (
          <div class="improve-report">
            <div class="settings-hint">
              {t("improve-scan-summary", { product: APP_PRODUCT_NAME, tineVersion: r().tineVersion, lsdocVersion: r().lsdocVersion, files: r().stats.files, bytes: fmtBytes(r().stats.totalBytes) })}
              <Show when={r().stats.skipped?.length}>{" "}<span title={r().stats.skipped?.join("\n")}>{t("improve-skipped-unreadable-or-oversized-file-s", { length: r().stats.skipped?.length ?? 0 })}</span>
              </Show>
            </div>

            <Show when={!r().lsdocAvailable}>
              <div class="improve-notice">{t("improve-divergence-detection-needs-the-next-lsdoc-build")}</div>
            </Show>

            <Show when={r().bench}>
              {(b) => (
                <div class="improve-bench"><h4>{t("improve-parse-speed-best-of-3")}</h4>
                  <table class="improve-table">
                    <thead><tr><th>{t("improve-parser")}</th><th>{t("improve-files")}</th><th>{t("improve-total")}</th><th>{t("improve-p50")}</th><th>{t("improve-p95")}</th><th>{t("improve-max")}</th></tr>
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
                <div class="improve-findings-head"><h4>{t("improve-divergences-2", { length: divergences().length })}</h4>
                  <Show when={divergences().length > 0}>
                    <button class="btn-secondary" onClick={() => flash(reportMarkdown(), "all")}>{copied() === "all" ? t("improve-copied") : copied() === "fail:all" ? t("improve-copy-failed") : t("improve-copy-all")}
                    </button>
                  </Show>
                </div>
                <Show when={divergences().length === 0}>
                  <div class="improve-clean">{t("improve-no-actionable-divergences-lsdoc-matched-logseq-s")}</div>
                </Show>
                <For each={divergences()}>
                  {(f) => (
                    <div class="improve-card">
                      <div class="improve-card-head">
                        <code>{f.rel}</code><span class="improve-lines">{t("improve-lines", { lineStart: f.lineStart, lineEnd: f.lineEnd })}</span>
                      </div><Show when={f.anonymized.ok ? f.anonymized : null} keyed fallback={<div class="settings-hint">{t("improve-found-but-couldn-t-be-anonymized-not")}</div>}>
                        {(a) => (
                          <><div class="settings-hint">{t("improve-anonymized-via", { tier: a.tier })}</div>
                            <pre class="improve-snippet">{a.input}</pre>
                            <button class="btn-secondary" onClick={() => flash(findingMarkdown(f), f.rel)}>{copied() === f.rel ? t("improve-copied") : copied() === `fail:${f.rel}` ? t("improve-copy-failed") : t("improve-copy")}
                            </button>
                          </>
                        )}
                      </Show>
                    </div>
                  )}
                </For>
                <Show when={oracleArtifacts().length > 0}>
                  <details class="improve-other"><summary>{t("improve-suppressed-mldoc-oracle-artifact-s", { length: oracleArtifacts().length })}</summary>
                    <For each={oracleArtifacts()}>
                      {(f) => (
                        <div class="settings-hint"><Rich text={t("improve-lines-2", { rel: richValue(f.rel), lineStart: richValue(f.lineStart), lineEnd: richValue(f.lineEnd), detail: richValue(f.detail) })} slots={{ "code": (children) => <code>{children}</code> }} /></div>
                      )}
                    </For>
                  </details>
                </Show>
                <Show when={intentionalDivergences().length > 0}>
                  <details class="improve-other"><summary>{t("improve-known-intentional-parser-difference-s-suppressed", { length: intentionalDivergences().length })}</summary>
                    <For each={intentionalDivergences()}>
                      {(f) => (
                        <div class="settings-hint"><Rich text={t("improve-lines-2", { rel: richValue(f.rel), lineStart: richValue(f.lineStart), lineEnd: richValue(f.lineEnd), detail: richValue(f.detail) })} slots={{ "code": (children) => <code>{children}</code> }} /></div>
                      )}
                    </For>
                  </details>
                </Show>
                <Show when={otherFindings().length > 0}>
                  <details class="improve-other"><summary>{t("improve-non-divergence-issue-s", { length: otherFindings().length })}</summary>
                    <For each={otherFindings()}>
                      {(f) => <div class="settings-hint"><code>{"rel" in f ? f.rel : ""}</code> — {f.type}</div>}
                    </For>
                  </details>
                </Show>
                <Show when={divergences().length > 0}><p class="settings-hint"><Rich text={t("improve-post-these-anonymized-snippets-to", { ISSUESURL: richValue(ISSUES_URL) })} slots={{ "a": (children) => <a href={ISSUES_URL} target="_blank" rel="noreferrer">{children}</a> }} /></p>
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
      <td>{props.name}</td><Show when={props.s} fallback={<td colSpan={5} class="settings-hint">{t("improve-pending-next-build")}</td>}>
        {(s) => (
          <>
            <td>{s().fileCount}</td><td>{t("improve-ms", { toFixed1: s().totalMs.toFixed(1) })}</td>
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
  return t("improve-phase", { phase: p });
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
