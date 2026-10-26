#!/usr/bin/env bash
# M4 availability gate (REVIEW-progress-plan R5): a 250- and a 1000-referrer
# rename through the page host are answered success, off Windows inside 10 s
# (OPERATION_WAIT), in a release build. The tests print the timings.
# Hosted Windows runs the same two tests in ci.yml's scope=windows job (G1). The structural linearity proof is the ordinary
# test rename_cost.rs rename_host_work_is_linear.
# Usage: scripts/rename-thousand-referrers.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
set +u; source scripts/env.sh >/dev/null; set -u
cargo test --release -p tine-graph-features --test integration -- \
  --ignored --exact --nocapture \
  rename_cost::a_250_referrer_rename_completes_inside_the_caller_bound \
  rename_cost::a_thousand_referrer_rename_completes_inside_the_caller_bound
