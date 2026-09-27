# og vs master native parity bench

Five runs per metric. Cells are median [min, max] in ms (RSS in MiB).

| Corpus | Metric | og | master | og vs master |
|---|---|---:|---:|---:|
| 2k | openMs | 1104.6 [1096.0, 1175.3] | 806.2 [791.6, 914.0] | 37.0% |
| 2k | openPageMs | 104.8 [64.8, 112.8] | 104.6 [56.8, 113.9] | 0.2% |
| 2k | typingP50Ms | 7.0 [6.0, 8.0] | 7.0 [6.0, 8.0] | 0.0% |
| 2k | typingP95Ms | 9.0 [8.0, 9.0] | 9.0 [8.0, 10.0] | 0.0% |
| 2k | saveMs | 523.0 [441.0, 539.0] | 468.0 [460.0, 513.0] | 11.8% |
| 2k | searchMs | 386.7 [345.3, 403.0] | 1347.0 [1194.5, 1383.4] | -71.3% |
| 2k | linkedReferencesMs | 63.7 [20.1, 69.1] | 1036.7 [984.0, 1190.0] | -93.9% |
| 2k | unlinkedReferencesMs | 65.5 [14.3, 68.8] | 65.6 [12.8, 96.5] | -0.3% |
| 2k | rename200Ms | 355.4 [350.1, 396.3] | 461.9 [394.8, 507.4] | -23.1% |
| 10k | openMs | 4298.2 [4248.2, 4370.6] | 958.8 [903.1, 987.3] | 348.3% |
| 10k | openPageMs | 95.3 [86.1, 133.6] | 109.7 [61.9, 112.9] | -13.1% |
| 10k | typingP50Ms | 6.0 [5.0, 6.0] | 7.0 [5.0, 8.0] | -14.3% |
| 10k | typingP95Ms | 8.0 [7.0, 9.0] | 9.0 [8.0, 9.0] | -11.1% |
| 10k | saveMs | 490.0 [426.0, 497.0] | 461.0 [459.0, 512.0] | 6.3% |
| 10k | searchMs | 566.7 [473.9, 575.1] | 5685.2 [5599.2, 5838.8] | -90.0% |
| 10k | linkedReferencesMs | 60.6 [14.4, 67.1] | 1189.0 [1073.4, 1227.3] | -94.9% |
| 10k | unlinkedReferencesMs | 55.8 [12.6, 63.3] | 230.1 [132.6, 240.4] | -75.7% |
| 10k | rename200Ms | 5603.2 [5544.2, 5690.0] | 843.5 [794.9, 910.7] | 564.3% |
| 10k | rssAfterOpenBytes | 948.3 [947.7, 949.8] | 221.3 [217.6, 223.6] | 328.4% |
| 10k | rssAfterJourneysBytes | 970.4 [967.8, 972.7] | 1146.9 [1142.6, 1152.5] | -15.4% |
| 10k | rssAfterRenameBytes | 970.4 [967.8, 972.7] | 1103.5 [1101.1, 1105.6] | -12.1% |
| anonymized | openMs | 708.0 [674.6, 820.9] | 747.8 [704.0, 780.1] | -5.3% |
| anonymized | openPageMs | 102.1 [56.2, 116.7] | 102.3 [67.7, 122.7] | -0.2% |
| anonymized | typingP50Ms | 8.0 [7.0, 8.0] | 7.0 [6.0, 9.0] | 14.3% |
| anonymized | typingP95Ms | 10.0 [9.0, 11.0] | 10.0 [10.0, 13.0] | 0.0% |
| anonymized | saveMs | 467.0 [463.0, 516.0] | 459.0 [443.0, 470.0] | 1.7% |
| anonymized | searchMs | 397.0 [350.1, 486.0] | 551.5 [495.5, 682.3] | -28.0% |
| anonymized | linkedReferencesMs | 20.2 [13.0, 75.9] | 81.6 [28.1, 238.2] | -75.3% |
| anonymized | unlinkedReferencesMs | 54.0 [13.1, 64.9] | 27.0 [12.1, 34.8] | 100.3% |
| anonymized | rename200Ms | 168.3 [132.4, 207.7] | 320.8 [245.0, 389.8] | -47.5% |

## Main-thread tasks or animation-frame gaps over 100 ms

WebKitGTK uses the animation-frame gap fallback on this runner. Values list every recorded gap over 100 ms across the five trials.

| Corpus | Journey | og maximum and gaps (ms) | master maximum and gaps (ms) |
|---|---|---|---|
| 2k | open | 159.0; 123.0, 127.0, 129.0, 121.0, 159.0 | 170.0; 105.0, 170.0, 101.0 |
| 2k | search | 0; none | 0; none |
| 2k | openPage | 0; none | 0; none |
| 2k | linkedReferences | 0; none | 0; none |
| 2k | unlinkedReferences | 0; none | 0; none |
| 2k | typing | 0; none | 0; none |
| 2k | save | 0; none | 0; none |
| 2k | rename | 0; none | 0; none |
| 10k | open | 3310.0; 169.0, 3257.0, 271.0, 175.0, 3291.0, 217.0, 117.0, 3266.0, 224.0, 121.0, 3310.0, 220.0, 166.0, 3251.0, 214.0 | 104.0; 104.0, 102.0, 102.0 |
| 10k | search | 0; none | 0; none |
| 10k | openPage | 0; none | 0; none |
| 10k | linkedReferences | 0; none | 0; none |
| 10k | unlinkedReferences | 0; none | 0; none |
| 10k | typing | 0; none | 0; none |
| 10k | save | 0; none | 0; none |
| 10k | rename | 0; none | 0; none |
| anonymized | open | 254.0; 121.0, 116.0, 122.0, 182.0, 254.0 | 165.0; 165.0, 108.0 |
| anonymized | search | 0; none | 0; none |
| anonymized | openPage | 0; none | 0; none |
| anonymized | linkedReferences | 0; none | 0; none |
| anonymized | unlinkedReferences | 0; none | 0; none |
| anonymized | typing | 0; none | 0; none |
| anonymized | save | 0; none | 0; none |
| anonymized | rename | 0; none | 0; none |
