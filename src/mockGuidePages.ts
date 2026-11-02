// Mock fixture pages for the level-colored indent guides (GH #856), shown with
// `?regressions`: eight levels of nesting, and the same outline embedded in a
// nested block (an embed starts its levels again at 1).
import type { BlockDto } from "./types";

/** Page names and their blocks; mock.ts wraps them as pages. */
export function guideLevelRegressionPages(b: (raw: string, children?: BlockDto[]) => BlockDto): [string, BlockDto[]][] {
  return [
    [
      "Guide level colors regression", [
        b("Level 0 root\nid:: ui-guide-levels-root", [
          b("Level 1 first", [
            b("Level 2 first", [
              b("Level 3 first", [
                b("Level 4 first", [
                  b("Level 5 first", [
                    b("Level 6 first", [b("Level 7 leaf"), b("Level 7 second leaf")]),
                    b("Level 6 second"),
                  ]),
                ]),
                b("Level 4 second"),
              ]),
            ]),
            b("Level 2 second", [b("Level 3 under second")]),
          ]),
          b("Level 1 second", [b("Level 2 under second")]),
        ]),
        b("Second root", [b("Child of second root", [b("Grandchild of second root")])]),
      ],
    ],
    [
      "Guide level colors embed regression",
      [b("Host", [b("Nested host", [b("{{embed ((ui-guide-levels-root))}}")])])],
    ],
  ];
}
