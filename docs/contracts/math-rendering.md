# Math rendering

Inline and display math use KaTeX without rewriting the authored TeX. KaTeX's
operator and relation break points remain available. When an indivisible formula
exceeds its pane, its own content scrolls horizontally so both ends are reachable.
Short inline formulas retain their baseline. Existing `.math`, `.math-display`
and KaTeX classes remain available to themes and `custom.css`.
Rows containing math retain native horizontal panning when touch gestures are
enabled; the block swipe handler yields to the overflowing formula.

This deliberately differs from OG's unconstrained KaTeX output (GH #915).
The shared app stylesheet covers live block surfaces, including split panes,
sidebar, embeds, query results and hover previews. PDF preparation also appends
the bundled stylesheet. Interactive containment applies only to screen media:
print retains unconstrained KaTeX layout without formula scrollport clipping.
Published HTML uses its own stylesheet. Fitting indivisible math to paper narrower
than its natural width is a separate print-policy question.

Containment is CSS-only: it adds no application file operations, stat calls,
fsyncs or IPC. The shared MathView typesets once per formula instance and once
per TeX/display-mode change; resizing panes and scrolling locally add zero
typesetting passes or component mounts (`src/render/mathOperations.test.tsx`).
Browser layout remains proportional to the rendered formula; there is no
graph-wide work or application resize listener for containment.
