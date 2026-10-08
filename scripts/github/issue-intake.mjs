const collaborators = new Set(["OWNER", "MEMBER", "COLLABORATOR"]);
export const CLOSE_COMMENT_WINDOW_MS = 30_000;

export function shouldReopen(event, issue, timeline) {
  const comment = event.comment;
  if (event.action !== "created" || issue.pull_request || issue.state !== "closed" ||
      !comment || comment.user?.type === "Bot" ||
      collaborators.has(comment.author_association)) return false;

  // Use API times, not webhook delivery/run times. Close-with-comment can record
  // either operation first, with second-resolution timestamps. Thirty seconds
  // covers the reported seconds-long race without suppressing later follow-ups.
  // Sort explicitly: don't depend on webhook or timeline response ordering.
  const transitions = timeline.filter((entry) =>
    entry.event === "closed" || entry.event === "reopened")
    .sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at) || a.id - b.id);
  const close = transitions.at(-1);
  const commentTime = Date.parse(comment.created_at);
  const closeTime = Date.parse(close?.created_at);
  if (close?.event !== "closed" || !Number.isFinite(commentTime) ||
      !Number.isFinite(closeTime) || !close.actor?.id || !comment.user?.id) return false;
  const elapsed = commentTime - closeTime;
  if (close.actor.id === comment.user.id &&
      Math.abs(elapsed) <= CLOSE_COMMENT_WINDOW_MS) return false;
  // A delayed delivery from before the current close must not undo that close.
  return elapsed >= 0;
}

function labelNames(issue) {
  return (issue.labels ?? []).map((label) =>
    (typeof label === "string" ? label : label.name).toLowerCase());
}

export function classificationLabel(issue) {
  if (issue.pull_request || labelNames(issue).some((name) =>
    name === "bug" || name === "enhancement")) return null;
  const prefix = /^\s*\[(bug|feature)\](?:\s|$)/i.exec(issue.title ?? "");
  const titleLabel = prefix ? (prefix[1].toLowerCase() === "bug" ? "bug" : "enhancement") : null;
  // GitHub forms have no title prefix today. Recognize their first level-three
  // heading, ignoring fenced examples, rather than keywords in ordinary prose.
  let fence = null;
  let firstHeading;
  for (const line of (issue.body ?? "").split(/\r?\n/)) {
    const marker = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (marker) {
      if (!fence) fence = marker[1];
      else if (marker[1][0] === fence[0] && marker[1].length >= fence.length &&
          line.trim() === marker[1]) fence = null;
      continue;
    }
    if (fence) continue;
    const heading = /^\s{0,3}###\s+(.+?)\s*$/.exec(line);
    if (heading) { firstHeading = heading[1]; break; }
  }
  const bodyLabel = firstHeading === "What happened?" ? "bug" :
    firstHeading === "What would you like Tine to do?" ? "enhancement" : null;
  if (titleLabel && bodyLabel && titleLabel !== bodyLabel) return null;
  return titleLabel ?? bodyLabel;
}

// github-script supplies Octokit and context; importing this file performs no I/O.
export async function reopenFollowup({ github, context }) {
  const event = context.payload;
  if (event.action !== "created" || event.issue.pull_request ||
      event.comment.user.type === "Bot" ||
      collaborators.has(event.comment.author_association)) return;
  const params = { ...context.repo, issue_number: event.issue.number };
  const { data: issue } = await github.rest.issues.get(params);
  if (issue.state !== "closed") return;
  const timeline = await github.paginate(github.rest.issues.listEventsForTimeline, {
    ...params, per_page: 100,
  });
  if (!shouldReopen(event, issue, timeline)) return;
  // Label first: if reopening fails, retry can finish; after success repeated
  // delivery sees an open issue and makes no writes. Never replace other labels.
  if (!labelNames(issue).includes("needs-triage")) {
    await github.rest.issues.addLabels({ ...params, labels: ["needs-triage"] });
  }
  await github.rest.issues.update({ ...params, state: "open" });
}

export async function labelOpenedIssue({ github, context }) {
  if (context.payload.action !== "opened" || context.payload.issue.pull_request) return;
  const params = { ...context.repo, issue_number: context.payload.issue.number };
  // Fresh labels protect explicit classification added after the opened event.
  const { data: issue } = await github.rest.issues.get(params);
  const label = classificationLabel(issue);
  if (label) await github.rest.issues.addLabels({ ...params, labels: [label] });
}
