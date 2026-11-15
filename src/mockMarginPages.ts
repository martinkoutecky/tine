// Mock fixture pages for the margin dialogue (vision 2026-10 §3.7), shown with
// `?regressions`: slice 1's comment cards and slice 2's stacking column.
import type { BlockDto } from "./types";

/** Page names and their blocks; mock.ts wraps them as pages (I-12 keeps page
 * construction in the mock backend and document/convert.ts). */
export function marginRegressionPages(b: (raw: string, children?: BlockDto[], collapsed?: boolean) => BlockDto): [string, BlockDto[]][] {
  return [
    ["Message outlines regression", [
      b("A passage worth discussing.", [
        b("The first message\ntine.quote:: passage\ntine.date:: 2026-11-07T13:37Z", [
          b("Evidence with [[references]]", [b("Nested evidence")]),
          b("A second message\ntine.date:: 2026-11-07T13:38Z\ntine.author:: Ada", [b("An editable reply outline")]),
        ]),
      ]),
    ]],
    [
      // GH #843: a block with both a reference count and a margin comment count.
      "Badge alignment regression", [
        b("The method section compares two approaches on the same survey.\nid:: f672c5bf-cba9-4704-93f4-06b22bc30cec", [
          b("Which survey? Name it here.\ntine.quote:: survey"),
        ]),
        b("((f672c5bf-cba9-4704-93f4-06b22bc30cec))"),
        b("# A heading with both badges\nid:: f672c5bf-cba9-4704-93f4-06b22bc30ced", [b("And its comment.\ntine.quote:: heading")]),
        b("((f672c5bf-cba9-4704-93f4-06b22bc30ced))"),
      ],
    ],
    [
      // GH #883: equal author names, with absent, one- and two-digit counts.
      "Author alignment regression", [
        b("Author alignment group\nid:: 88300000-0000-4000-8000-000000000001", [
          b("Unreferenced passage.\ntine.author:: Ada\nid:: 88300000-0000-4000-8000-000000000002"),
          b("Referenced passage with enough ordinary words to wrap in a narrow pane and keep both metadata pills on its first line.\ntine.author:: Ada\nid:: 88300000-0000-4000-8000-000000000003"),
          b("DONE A passage referenced twelve times.\ntine.author:: Ada\nid:: 88300000-0000-4000-8000-000000000004"),
        ]),
      ],
    ],
    [
      "Author alignment referrers", [
        b("One reference ((88300000-0000-4000-8000-000000000003))"),
        ...Array.from({ length: 12 }, (_, i) => b(`Reference ${i + 1} ((88300000-0000-4000-8000-000000000004))`)),
      ],
    ],
    [
      // GH #893: mixed owner/authored blocks, a folded authored block, nesting,
      // a numbered block and live copies (block embed, linked reference).
      "Author mark gap", [
        b("You wrote this block, so it has no author mark."),
        b("An agent wrote this block. Its first letter starts where yours does.\ntine.author:: Ada"),
        b("A second agent block in the same run, long enough to wrap onto another line in a narrow window.\ntine.author:: Ada"),
        b("Back to your own words."),
        b("A folded agent block\ntine.author:: Bo\ncollapsed:: true\nid:: 89300000-0000-4000-8000-000000000001", [b("Hidden child")], true),
        b("Your parent block", [
          b("An agent child, one level in\ntine.author:: Ada", [
            b("An agent grandchild, two levels in\ntine.author:: Bo"),
            b("Your grandchild"),
          ]),
          b("Your child"),
        ]),
        b("A numbered agent block\ntine.author:: Ada\nlogseq.order-list-type:: number"),
        b("An agent block that mentions [[Author mark refs]]\ntine.author:: Bo\nid:: 89300000-0000-4000-8000-000000000002"),
      ],
    ],
    [
      "Author mark copies", [
        b("{{embed ((89300000-0000-4000-8000-000000000002))}}"),
      ],
    ],
    ["Author mark refs", [b("This page is mentioned by an agent block.")]],
    [
      // GH #893: an authored block that also carries a margin comment on a passage of its text.
      "Author tint comment", [
        b("You wrote the line above."),
        b("An agent drafted this passage worth discussing, and a reader commented on it.\ntine.author:: Ada", [
          b("Which passage? Name it.\ntine.quote:: passage worth discussing\ntine.date:: 2026-11-07T13:37Z"),
        ]),
        b("A second agent block next to the commented one.\ntine.author:: Ada"),
      ],
    ],
    [
      "Author alignment embeds", [
        b("{{embed ((88300000-0000-4000-8000-000000000001))}}"),
        b("{{embed [[Author alignment regression]]}}"),
      ],
    ],
    [
      // GH #110: the first comment on a block with a long existing subtree.
      "Margin creation regression", [
        ...Array.from({ length: 12 }, (_, i) => b(`Opening paragraph ${i + 1}.`)),
        b("Discuss these words here", Array.from({ length: 80 }, (_, i) => b(`Existing child ${i + 1}.`))),
        b("Closing paragraph."),
      ],
    ],
    [
      // Margin dialogue slice 1 (vision §3.7): an agent block, a comment with a
      // three-level thread (slice 2b: a depth-3 reply shows the indent cap), a stale quote and a repeated-phrase quote.
      "Margin comments regression", [
        b("Gradient descent converges here because the step size shrinks, and the step size shrinks geometrically, so the error halves each round.\ntine.author:: claude", [
          b("Only if the loss is convex; say so.\ntine.quote:: converges here because", [
            b("Fair: the lemma assumes convexity. I will add it.\ntine.author:: claude", [
              b("Thanks. Also cite the source.", [
                b("Done: it is in the footnote.\ntine.author:: claude"),
              ]),
            ]),
          ]),
          b("Which of the two do you mean?\ntine.quote:: the step size shrinks\ntine.quote-prefix:: the step size shrinks, and\ntine.quote-suffix:: geometrically, so the error"),
          b("This sentence was rewritten since.\ntine.quote:: the rate is linear in the dimension"),
        ]),
        b("A block I wrote myself, with no comments."),
      ],
    ],
    [
      // Margin dialogue slice 2: six comments on one paragraph stack downward
      // in the margin without overlapping.
      "Margin stacking regression", [
        b("An opening block without comments, so the commented paragraph is not first.", [b("A child of it.")]),
        b("The proof bounds the error by the gradient norm, then sums the bounds over all rounds, and the sum telescopes because each step only shrinks the potential; the constant depends on the smoothness of the loss.\ntine.author:: claude", [
          b("Which error, the training or the test error?\ntine.quote:: bounds the error"),
          b("Name the norm.\ntine.quote:: gradient norm"),
          b("Over all rounds or only the first T?\ntine.quote:: over all rounds", [b("All rounds; T is the horizon.\ntine.author:: claude")]),
          b("Does it really telescope here?\ntine.quote:: the sum telescopes"),
          b("Shrinks by how much per step?\ntine.quote:: only shrinks the potential"),
          b("Smoothness constant L, say so.\ntine.quote:: smoothness of the loss"),
        ]),
        b("A closing block after the commented paragraph."),
      ],
    ],
    [
      // MARGIN-POS (2026-10-31): a comment on a block with a deep subtree (GH
      // #110: inline, it must show right under the block, not after "def"); a
      // passage on a later line of a wrapped paragraph, with replies by two other
      // authors; and an adjacent block whose thread the tall one pushes down.
      "Margin position regression", [
        b("123 needs a source", [
          b("456"),
          b("789"),
          b("abc", [b("def")]),
          b("Which source?\ntine.quote:: needs a source\ntine.author:: Ellis"),
        ]),
        b("This paragraph runs long enough to wrap onto several lines in the reading column, so that the passage the comment quotes sits well below the top of its block, which is where the thread must stay when the block is edited and when it is not.", [
          b("Say which block.\ntine.quote:: when the block is edited", [
            b("The commented one.\ntine.author:: claude"),
            b("Agreed.\ntine.author:: Codex"),
            b("Then I will reword it."),
          ]),
        ]),
        b("A short block right below.", [b("Short?\ntine.quote:: right below")]),
      ],
    ],
  ];
}
