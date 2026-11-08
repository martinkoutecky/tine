# Code wrapping and reading position

The device preference **Wrap code lines**, its palette command **Toggle code
block wrap**, and a user-assigned shortcut share `changeCodeWrapping`. Both
directions preserve the reading position in mounted main, split and right
sidebar surfaces, with either editor selected. Note source is unchanged.

The shared reading anchor prefers the first block whose start is in the
viewport. A clipped preceding code card must not push that visible block away
when its own height changes. Outline containers include their descendants, so
an offscreen parent is not a substitute for the visible descendant. If no
block starts in the viewport (for example, a tall code card fills it), anchor
the intersecting block. A focused descendant retains priority. Resize
observation covers late layout and shrinking scroll-range clamping. Gestures,
focus changes, other scroll owners and retiring content release the anchor.

Proof: `Settings.codeDisplay.test.tsx`, `CodeWrap.viewport.test.tsx`,
`readingViewportAnchor.test.tsx`, and `scripts/e2e-code-wrap-toggle.mjs`.
The native journey checks that its reading marker is visible and unobscured
before and after toggling; offsets of offscreen or covered markers are not
reading-position evidence. It exercises palette, binding and Settings with
both editor preferences in all three surfaces.
