/** The standard share glyph (three linked nodes), shared by the topbar, the page title and query blocks. */
export function ShareIcon(props: { class?: string }) {
  return (
    <svg viewBox="0 0 24 24" class={props.class ?? "nav-icon"} aria-hidden="true">
      <circle cx="18" cy="5.5" r="2.6" fill="none" stroke="currentColor" stroke-width="1.7" />
      <circle cx="6" cy="12" r="2.6" fill="none" stroke="currentColor" stroke-width="1.7" />
      <circle cx="18" cy="18.5" r="2.6" fill="none" stroke="currentColor" stroke-width="1.7" />
      <line x1="8.3" y1="10.7" x2="15.7" y2="6.8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
      <line x1="8.3" y1="13.3" x2="15.7" y2="17.2" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
    </svg>
  );
}
