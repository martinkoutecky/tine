# og vs master native parity bench

Five runs per metric. Cells are median [min, max] in ms (RSS in MiB).

| Corpus | Metric | og | master | og vs master |
|---|---|---:|---:|---:|
| 2k | openMs | 1134.1 [1124.4, 1237.5] | 795.9 [789.5, 853.0] | 42.5% |
| 2k | openPageMs | 176.0 [94.4, 201.1] | 119.8 [84.5, 137.0] | 47.0% |
| 2k | typingP50Ms | 7.0 [4.0, 8.0] | 6.0 [5.0, 8.0] | 16.7% |
| 2k | typingP95Ms | 9.0 [8.0, 11.0] | 10.0 [8.0, 11.0] | -10.0% |
| 2k | saveMs | failed (0/5 samples) | 474.0 [464.0, 511.0] | — |
| 2k | searchMs | 364.4 [317.2, 437.0] | 1191.2 [1060.5, 1355.0] | -69.4% |
| 2k | linkedReferencesMs | 60.3 [24.7, 73.6] | 1041.0 [1026.8, 1088.3] | -94.2% |
| 2k | unlinkedReferencesMs | 29.9 [27.3, 67.1] | 30.0 [12.1, 61.4] | -0.4% |
| 2k | rename200Ms | failed (0/5 samples) | 725.2 [722.6, 905.9] | — |
| 10k | openMs | 4240.8 [4219.6, 4341.9] | 969.0 [916.2, 1040.3] | 337.6% |
| 10k | openPageMs | 206.0 [120.8, 233.5] | 86.6 [71.4, 150.7] | 137.9% |
| 10k | typingP50Ms | 6.0 [5.0, 7.0] | 8.0 [6.0, 8.0] | -25.0% |
| 10k | typingP95Ms | 8.0 [8.0, 9.0] | 9.0 [8.0, 10.0] | -11.1% |
| 10k | saveMs | failed (0/5 samples) | 495.0 [457.0, 504.0] | — |
| 10k | searchMs | 591.4 [548.5, 641.1] | 5689.2 [5442.0, 6080.0] | -89.6% |
| 10k | linkedReferencesMs | 60.5 [29.6, 103.8] | 1183.0 [1033.3, 1283.2] | -94.9% |
| 10k | unlinkedReferencesMs | 63.9 [12.9, 105.0] | 145.0 [132.9, 328.2] | -55.9% |
| 10k | rename200Ms | failed (0/5 samples) | 3028.3 [2992.4, 3122.5] | — |
| 10k | rssAfterOpenBytes | 941.1 [940.5, 942.3] | 206.7 [203.2, 210.7] | 355.4% |
| 10k | rssAfterJourneysBytes | 948.4 [948.1, 948.4] | 1131.3 [1120.1, 1138.0] | -16.2% |
| 10k | rssAfterRenameBytes | 948.2 [947.7, 948.4] | 868.0 [842.5, 877.6] | 9.2% |
| anonymized | openMs | 685.3 [674.5, 707.6] | 750.2 [718.7, 781.9] | -8.7% |
| anonymized | openPageMs | 130.5 [118.7, 165.3] | 118.3 [99.9, 144.5] | 10.3% |
| anonymized | typingP50Ms | 7.0 [6.0, 8.0] | 8.0 [6.0, 9.0] | -12.5% |
| anonymized | typingP95Ms | 10.0 [7.0, 12.0] | 11.0 [8.0, 11.0] | -9.1% |
| anonymized | saveMs | failed (0/5 samples) | 503.0 [430.0, 516.0] | — |
| anonymized | searchMs | 384.0 [334.7, 446.8] | 529.1 [525.2, 679.3] | -27.4% |
| anonymized | linkedReferencesMs | 65.1 [29.5, 116.1] | 173.5 [61.5, 263.3] | -62.5% |
| anonymized | unlinkedReferencesMs | 61.5 [14.6, 104.4] | 13.5 [12.4, 68.8] | 355.4% |
| anonymized | rename200Ms | failed (0/5 samples) | 609.6 [353.4, 682.2] | — |

## Main-thread tasks or animation-frame gaps over 100 ms

WebKitGTK uses the animation-frame gap fallback on this runner. Values list every recorded gap over 100 ms across the five trials.

| Corpus | Journey | og maximum and gaps (ms) | master maximum and gaps (ms) |
|---|---|---|---|
| 2k | open | 265.0; 123.0, 166.0, 265.0, 122.0, 128.0, 110.0 | 163.0; 101.0, 101.0, 163.0 |
| 2k | search | 0; none | 0; none |
| 2k | openPage | 0; none | 0; none |
| 2k | linkedReferences | 0; none | 0; none |
| 2k | unlinkedReferences | 0; none | 0; none |
| 2k | typing | 0; none | 0; none |
| 2k | save | 0; none | 0; none |
| 2k | rename | 0; none | 0; none |
| 10k | open | 3310.0; 123.0, 3310.0, 261.0, 128.0, 3236.0, 255.0, 125.0, 3244.0, 257.0, 140.0, 3161.0, 240.0, 179.0, 3185.0, 288.0 | 112.0; 105.0, 102.0, 112.0, 107.0 |
| 10k | search | 0; none | 0; none |
| 10k | openPage | 0; none | 0; none |
| 10k | linkedReferences | 0; none | 0; none |
| 10k | unlinkedReferences | 0; none | 0; none |
| 10k | typing | 0; none | 0; none |
| 10k | save | 0; none | 0; none |
| 10k | rename | 0; none | 0; none |
| anonymized | open | 190.0; 176.0, 131.0, 164.0, 190.0, 176.0 | 165.0; 103.0, 165.0 |
| anonymized | search | 0; none | 0; none |
| anonymized | openPage | 0; none | 0; none |
| anonymized | linkedReferences | 0; none | 0; none |
| anonymized | unlinkedReferences | 0; none | 0; none |
| anonymized | typing | 0; none | 0; none |
| anonymized | save | 0; none | 0; none |
| anonymized | rename | 0; none | 0; none |
