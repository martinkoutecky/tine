
# Interface text; variables carry untranslated names, paths or diagnostic details.
improve-tine-s-parser-lsdoc-is-a-from-scratch-reimplementation = Tine's parser (<strong>lsdoc</strong>) is a from-scratch reimplementation of the parser Logseq itself uses ( <strong1>mldoc</strong1>). This runs both on <strong2>your</strong2> graph, entirely on this machine, and finds any place they disagree — which is how you can help make Tine render your notes exactly like Logseq.
improve-privacy-nothing-is-uploaded-every-divergence-snippet = <strong>Privacy:</strong> nothing is uploaded. Every divergence snippet shown below is <strong1>anonymized</strong1> (page names and words replaced; URL schemes kept but hosts and paths scrubbed) and <strong2>re-checked</strong2> to confirm it still reproduces the bug. Tine omits a finding when that privacy-preserving scrub loses the mismatch. Still read every snippet carefully before you choose to share it.
improve-what-to-run = What to run
improve-both = Both
improve-divergences = Divergences
improve-speed = Speed
improve-include-journals = Include journals
improve-fast-scan = Fast scan
improve-faster-but-can-miss-or-invent-divergences = Faster, but can miss or invent divergences; only confirmed ones are shown. Leave off for a thorough, authoritative scan.
improve-running = Running…
improve-run-comparison = Run comparison
improve-failed = Failed: { $error }




# English retains the existing "(s)" wording; translations may inflect by count.
improve-skipped-unreadable-or-oversized-file-s = { $length ->
    [one] Skipped { $length } unreadable or oversized file(s).
   *[other] Skipped { $length } unreadable or oversized file(s).
    }
improve-divergence-detection-needs-the-next-lsdoc-build = Divergence detection needs the next lsdoc build (0.3.4) — this build can't yet parse whole files with lsdoc in-app. Speed numbers for Logseq's parser are shown below; the lsdoc column and divergence scan will light up after the update.
improve-parse-speed-best-of-3 = Parse speed (best of 3)
improve-parser = Parser
improve-files = Files
improve-total = Total
improve-p50 = p50
improve-p95 = p95
improve-max = max
improve-divergences-2 = Divergences ({ $length })
improve-copied = Copied!
improve-copy-failed = Copy failed
improve-copy-all = Copy all
improve-no-actionable-divergences-lsdoc-matched-logseq-s = No actionable divergences — lsdoc matched Logseq's parser on every file after verified mldoc oracle artifacts were quarantined.
improve-lines = lines { $lineStart }-{ $lineEnd }
improve-found-but-couldn-t-be-anonymized-not = Found, but couldn't be anonymized — not shown.
improve-anonymized-via = anonymized via { $tier }
improve-copy = Copy
improve-suppressed-mldoc-oracle-artifact-s = { $length } suppressed mldoc oracle artifact(s)
improve-lines-2 = <code>{ $rel }</code> · lines { $lineStart }-{ $lineEnd } — { $detail }
# English retains the existing "(s)" wording; translations may inflect by count.
improve-known-intentional-parser-difference-s-suppressed = { $length ->
    [one] { $length } known intentional parser difference(s) suppressed
   *[other] { $length } known intentional parser difference(s) suppressed
    }
improve-non-divergence-issue-s = { $length } non-divergence issue(s)
improve-post-these-anonymized-snippets-to = Post these anonymized snippets to <a>{ $ISSUESURL }</a>
improve-pending-next-build = pending next build
improve-ms = { $toFixed1 } ms


# English retains the existing "(s)" wording; translations may inflect by count.
improve-scan-summary = { $files ->
    [one] { $product } { $tineVersion } · lsdoc { $lsdocVersion } · Scanned { $files } file(s), { $bytes }.
   *[other] { $product } { $tineVersion } · lsdoc { $lsdocVersion } · Scanned { $files } file(s), { $bytes }.
    }
improve-phase = { $phase ->
    [scan] Scanning
    [verify] Verifying
   *[bench] Timing
    }
