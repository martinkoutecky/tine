// Mock fixture pages for the margin dialogue (vision 2026-10 §3.7), shown with
// `?regressions`: slice 1's comment cards and slice 2's stacking column.
import type { BlockDto } from "./types";

/** Page names and their blocks; mock.ts wraps them as pages (I-12 keeps page
 * construction in the mock backend and document/convert.ts). */
export function marginRegressionPages(b: (raw: string, children?: BlockDto[]) => BlockDto): [string, BlockDto[]][] {
  return [
    [
      // Margin dialogue slice 1 (vision §3.7): an agent block, a comment with a
      // two-level thread, a stale quote and a repeated-phrase quote.
      "Margin comments regression", [
        b("Gradient descent converges here because the step size shrinks, and the step size shrinks geometrically, so the error halves each round.\nauthor:: claude", [
          b("Only if the loss is convex; say so.\nquote:: converges here because", [
            b("Fair: the lemma assumes convexity. I will add it.\nauthor:: claude", [
              b("Thanks. Also cite the source."),
            ]),
          ]),
          b("Which of the two do you mean?\nquote:: the step size shrinks\nquote-prefix:: the step size shrinks, and\nquote-suffix:: geometrically, so the error"),
          b("This sentence was rewritten since.\nquote:: the rate is linear in the dimension"),
        ]),
        b("A block I wrote myself, with no comments."),
      ],
    ],
    [
      // Margin dialogue slice 2: six comments on one paragraph stack downward
      // in the margin without overlapping.
      "Margin stacking regression", [
        b("An opening block without comments, so the commented paragraph is not first.", [b("A child of it.")]),
        b("The proof bounds the error by the gradient norm, then sums the bounds over all rounds, and the sum telescopes because each step only shrinks the potential; the constant depends on the smoothness of the loss.\nauthor:: claude", [
          b("Which error, the training or the test error?\nquote:: bounds the error"),
          b("Name the norm.\nquote:: gradient norm"),
          b("Over all rounds or only the first T?\nquote:: over all rounds", [b("All rounds; T is the horizon.\nauthor:: claude")]),
          b("Does it really telescope here?\nquote:: the sum telescopes"),
          b("Shrinks by how much per step?\nquote:: only shrinks the potential"),
          b("Smoothness constant L, say so.\nquote:: smoothness of the loss"),
        ]),
        b("A closing block after the commented paragraph."),
      ],
    ],
  ];
}
