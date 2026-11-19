export function ThreadJoinMasks() {
  return (
    <svg id="thread-join-masks" width="0" height="0" style={{ position: "absolute" }} aria-hidden="true">
      <mask id="thread-rail-mask" maskContentUnits="objectBoundingBox" x="-1" y="0" width="3" height="1" shape-rendering="crispEdges">
        <rect width="1" height="1" fill="white" />
      </mask>
      <mask id="thread-arm-mask" maskContentUnits="objectBoundingBox" x="-1" y="-1" width="3" height="3" shape-rendering="crispEdges">
        <rect y="-1" width="1" height="3" fill="white" />
      </mask>
      <mask id="thread-elbow-mask" x="-1" y="0" width="3" height="2">
        {/* Clip the 1px paint extension at the top, leaving the curve and sides antialiased. */}
        <rect x="-10000" y="1" width="20000" height="10000" fill="white" shape-rendering="crispEdges" />
      </mask>
    </svg>
  );
}
