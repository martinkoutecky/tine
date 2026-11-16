// Mock fixture pages for the margin dialogue (vision 2026-10 §3.7), shown with
// `?regressions`: slice 1's comment cards and slice 2's stacking column.
import type { BlockDto } from "./types";

/** Page names and their blocks; mock.ts wraps them as pages (I-12 keeps page
 * construction in the mock backend and document/convert.ts). */
export function marginRegressionPages(b: (raw: string, children?: BlockDto[], collapsed?: boolean) => BlockDto): [string, BlockDto[], "org"?][] {
  return [
    ["Org label brackets 966", [b("A [[git]] tool for [[Emacs][my favorite editor]]")], "org"],
    ["Rendering regression", [
      b("Numbered fold markers", Array.from({ length: 12 }, (_, i) =>
        b(`Numbered item ${i + 1}\nlogseq.order-list-type:: number\ncollapsed:: true`, [b("Hidden child")], true))),
      b("Threaded parent with a passage", [
        b("First outline child", [b("A deeper branch", [b("Deep leaf")])]),
        b("Last outline child"),
        b("A margin message\ntine.quote:: passage\ntine.date:: 2026-11-07T13:37Z", [
          b("A reply message\ntine.date:: 2026-11-07T13:38Z", [b("An outline within the reply", [b("Reply outline leaf")])]),
          b("Supporting outline", [b("Supporting leaf")]),
        ]),
        b("An undated comment\ntine.quote:: Threaded parent", [
          b("An undated reply", [b("A nested undated reply")]),
        ]),
      ]),
      b("[[Rendering regression/Child]]"),
    ]],
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
      // GH #892: runs across depth, a break by your own block, a second author, a folded authored block.
      "Author thread", [
        b("You wrote the introduction."),
        b("Claude drafted this section.\ntine.author:: Claude", [
          b("First point, one level in.\ntine.author:: Claude", [b("A detail under the first point.\ntine.author:: Claude")]),
          b("Second point.\ntine.author:: Claude"),
        ]),
        b("Claude's closing line for the section, after its children.\ntine.author:: Claude"),
        b("Your reply, which ends the thread."),
        b("Mira reviewed the draft.\ntine.author:: Mira", [
          b("Mira's note on the second point.\ntine.author:: Mira"),
          b("A folded note by Mira.\ntine.author:: Mira\ncollapsed:: true\nid:: 89200000-0000-4000-8000-000000000001", [b("Hidden child")], true),
        ]),
        b("Mira's last word.\ntine.author:: Mira"),
      ],
    ],
    [
      // GH #892 review F2: one run of rows whose trailing comment controls differ: a plain row, a stale-quote
      // marker (phone: the open-comment marker), a resolved label and a folded block's "N below" chip.
      "Author thread trailing", [
        b("Claude's plain row.\ntine.author:: Claude"),
        b("Claude's row with a comment marker.\ntine.author:: Claude", [
          b("A passage that no longer exists.\ntine.quote:: words that were rewritten\ntine.author:: Ellis"),
        ]),
        b("Claude's row with a resolved comment.\ntine.author:: Claude", [
          b("Settled.\ntine.quote:: resolved comment\ntine.resolved:: true\ntine.author:: Ellis"),
        ]),
        b("Claude's folded row with a comment below.\ntine.author:: Claude\ncollapsed:: true\nid:: 89200000-0000-4000-8000-000000000002", [
          b("A hidden child\ntine.author:: Claude", [
            b("A hidden comment.\ntine.quote:: hidden child\ntine.author:: Ellis"),
          ]),
        ], true),
        b("Claude's closing row.\ntine.author:: Claude"),
      ],
    ],
    [
      // GH #892 review round 2: long author names. Every authored row starts a run (an owner row sits between), has a
      // long first line, and sits beside one kind of trailing control: none, a comment marker, a resolved label, a
      // folded block's "N below" chip.
      "Author thread long names", [
        b("An owner row with a first line long enough to run across the whole reading column and wrap onto a second line, as a paragraph does, so the text column of an unauthored row can be measured against the authored ones."),
        b("Alexandria wrote a first line long enough to run across the whole reading column and wrap onto a second line, as a paragraph does, right where the author's name is drawn.\ntine.author:: Alexandria"),
        b("An owner row between the runs."),
        b("Alexandria Montgomery-Winterbourne wrote a first line long enough to run across the whole column, beside a comment marker.\ntine.author:: Alexandria Montgomery-Winterbourne", [
          b("A passage that no longer exists.\ntine.quote:: words that were rewritten\ntine.author:: Ellis"),
        ]),
        b("An owner row between the runs."),
        b("Alexandria wrote a first line long enough to run across the whole column, beside a resolved comment.\ntine.author:: Alexandria", [
          b("Settled.\ntine.quote:: resolved comment\ntine.resolved:: true\ntine.author:: Ellis"),
        ]),
        b("An owner row between the runs."),
        b("Alexandria wrote a first line long enough to run across the whole column, folded with a comment below.\ntine.author:: Alexandria\ncollapsed:: true\nid:: 89200000-0000-4000-8000-000000000003", [
          b("A hidden child\ntine.author:: Alexandria", [
            b("A hidden comment.\ntine.quote:: hidden child\ntine.author:: Ellis"),
          ]),
        ], true),
        // An unauthored row with a fold button (phone: the 44px area at the row's end), folded so its child stays hidden.
        b("An owner row with a fold button and a first line long enough to run across the whole reading column.\ncollapsed:: true", [b("A hidden owner child.")], true),
        b("An owner row to close."),
      ],
    ],
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
