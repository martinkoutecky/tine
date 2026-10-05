/** The focused surface as an async owner (I-20). Work that begins from a user
 * action on the surface in front of them (a home-page read, a follow-link
 * resolution, a jump to a mention) may land only while that same surface is
 * still in front: the focused pane's router, its active tab, its route-intent
 * revision and its route must all be unchanged, so an A→B→A navigation or a
 * focus move to another pane or tab showing an equal route retires the work.
 * The graph binding is captured too. Capture it at the moment the user's intent
 * is final (after any navigation the work itself performs). Construction is
 * O(route fields); each check is O(number of extra predicates). Exemplar:
 * `openConfiguredHomePage` in homePage.ts. */
import { graphOwner, type Owner } from "./owned";
import { focusedRouter } from "./panes";
import { sameRoute } from "./router";

export function focusedSurfaceOwner(...live: Owner[]): Owner {
  const router = focusedRouter();
  const tabId = router.activeId();
  const intent = router.routeIntentRevision();
  const startingRoute = { ...router.route() };
  return graphOwner(
    () => focusedRouter() === router && router.activeId() === tabId
      && router.routeIntentRevision() === intent && sameRoute(router.route(), startingRoute),
    ...live,
  );
}
