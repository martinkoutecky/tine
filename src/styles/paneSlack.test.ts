import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const app = readFileSync("src/styles/app.css", "utf8");
const appTsx = readFileSync("src/App.tsx", "utf8");

function ruleBody(selectorPattern: RegExp): string {
  return app.match(selectorPattern)?.[1] ?? "";
}

// GH #369: the end-of-page editing slack used to be `padding-bottom: 40vh` on
// .main-content-inner — 40% of the WINDOW inside every pane scroller. In a
// split pane a fraction of the window tall, content + 40vh still overflowed
// the pane, so short panes whose content fit showed a useless vertical
// scrollbar (the reporter's "dashboard"). The slack is now a flex spacer
// inside the pane's own scroller, so it measures against the pane, not the
// window: fitting panes show no bar, long panes keep their breathing room.
describe("pane end-of-page slack is pane-relative (GH #369)", () => {
  it("sizes the editing slack from the pane's own scroller, not the viewport (vh)", () => {
    // No viewport-relative vertical slack may be declared for the page column
    // anywhere: not on the inner wrapper, not in the mobile media override.
    const appNoComments = app.replace(/\/\*[^]*?\*\//g, "");
    expect(appNoComments).not.toMatch(/\.main-content-inner\s*\{[^}]*\b\d+vh\b/);
    expect(appNoComments).not.toMatch(/\.main-content-inner\b[^}]*\bpadding[^;]*vh\b/);
  });

  it("makes the pane scroller a column flex context and keeps it scrolling", () => {
    const rule = ruleBody(/^\.main-content\s*\{([^}]*)\}/m);
    expect(rule).toContain("display: flex");
    expect(rule).toContain("flex-direction: column");
    expect(rule).toMatch(/overflow-y:\s*auto/);
  });

  it("keeps the page column at natural height so only real overflow can scroll", () => {
    const inner = ruleBody(/^\.main-content-inner\s*\{([^}]*)\}/m);
    // As a flex item of the scroller, the column must never be compressed:
    // shrinking it would fake "content fits" while clipping the page tail.
    expect(inner).toContain("flex: 0 0 auto");
  });

  it("stretches the page column to the configured reading width instead of its current contents", () => {
    const inner = ruleBody(/^\.main-content-inner\s*\{([^}]*)\}/m);
    // A flex item with auto side margins and only max-width shrink-wraps to its
    // children. Entering an editor or expanding references then changes the
    // entire page width (GH #382).
    expect(inner).toMatch(/\bwidth:\s*100%/);
  });

  it("caps the standard column at the user's override, else the theme default; Wide mode likewise", () => {
    const inner = ruleBody(/^\.main-content-inner\s*\{([^}]*)\}/m);
    expect(inner).toContain("max-width: var(--tine-main-content-max-width, var(--ls-main-content-max-width))");
    const wide = ruleBody(/^\.wide-mode \.main-content-inner\s*\{([^}]*)\}/m);
    expect(wide).toContain("max-width: var(--tine-wide-content-max-width, var(--ls-main-content-max-width-wide))");
  });

  it("lets the idle spacer absorb only real free space, never manufacture overflow", () => {
    const spacer = ruleBody(/^\.main-content::after\s*\{([^}]*)\}/m);
    // Grow through unused pane space, but start from zero and remain shrinkable:
    // an idle page that naturally fits at any occupancy must have no range.
    expect(spacer).toContain('content: ""');
    expect(spacer).toMatch(/flex:\s*1\s+1\s+0/);
  });

  it("keeps 40%-of-pane tail room stable for naturally overflowing pages (GH #390)", () => {
    const overflowing = ruleBody(/^\.main-content\.natural-content-overflow::after\s*\{([^}]*)\}/m);
    expect(overflowing).toMatch(/flex:\s*1\s+0\s+40%/);
    // Mounting a textarea is transient and must not change pane geometry.
    expect(app).not.toMatch(/\.main-content:has\(\.block-editor\)::after/);
  });

  it("toggles that class from the pane's measured natural overflow, in every pane scroller (GH #390)", () => {
    expect(appTsx).toMatch(/"natural-content-overflow":\s*naturalOverflow\(\)/);
    expect(appTsx).toMatch(/inner\.scrollHeight\s*>\s*scroller\.clientHeight\s*\+\s*1/);
    // Both the solo and the multi-pane leaf render through PaneScroller.
    expect(appTsx).not.toMatch(/<main\s+class="main-content/);
  });
});
