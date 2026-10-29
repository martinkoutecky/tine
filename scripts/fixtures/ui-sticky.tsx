// Production SeenHeader in page, split-pane and sidebar scroll shells.
import { render } from "solid-js/web";
import { SeenHeader } from "../../src/seen/SeenHeader";
import type { PageSeen } from "../../src/seen/pageSeen";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";

const params = new URLSearchParams(location.search);
const seen: PageSeen = {
  count: () => 3, identityKnown: () => false, unattributed: () => true, reason: () => null,
  changed: () => false, removedAfter: () => [], removedFirst: () => [],
  position: () => (params.get("step") ? { index: 12, total: 12, wrapped: true } : null), step: async () => {},
};
document.documentElement.setAttribute("data-theme", params.get("theme") || "light");
render(() => <div style={{ display: "flex", height: "100vh", gap: "12px", padding: "12px", "box-sizing": "border-box" }}>
  {(params.get("panes") === "all" ? ["main-content", "main-content", "right-sidebar"] : ["main-content"]).map((shell, i) =>
    <div class={shell} style={{ flex: "1 1 0", "min-width": "0", width: "auto", "max-width": "none", padding: "0 12px", height: "100%", "overflow-y": "auto" }}>
      <div class="page-section">
        <h1 class="page-title">{i === 2 ? "Sidebar fixture" : "Sticky notice"}</h1>
        <SeenHeader seen={seen} pageName="Fixture" />
        {Array.from({ length: 80 }, (_, n) => <div class="block-main" style={{ padding: "10px 0" }}>Visible row {n}</div>)}
      </div>
    </div>)}
</div>, document.getElementById("fixture")!);
