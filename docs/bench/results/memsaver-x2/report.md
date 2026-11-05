
### PSS of the whole app, MiB, median of the runs (n in brackets)

| App | Mode | n | Start, 15 s | Front, idle 60 s | Front, typing 60 s | Behind 1 min | Behind 10 min | Back in front, 20 s |
|---|---|---|---|---|---|---|---|---|
| Discord | stock | 3 | 639 | 540 | 546 | 283 | 438 | 442 |
| Discord | shipped | 3 | 473 | 15 | 51 | 33 | 28 | 35 |
| Discord | shipped-nopageout | 3 | 569 | 419 | 45 | 36 | 35 | 42 |
| VS Code (Code - OSS) | stock | 3 | 1086 | 882 | 814 | 296 | 423 | 439 |
| VS Code (Code - OSS) | shipped | 3 | 821 | 761 | 691 | 215 | 317 | 321 |
| Signal | stock | 3 | 589 | 491 | 491 | 289 | 306 | 306 |
| Signal | shipped | 3 | 508 | 409 | 409 | 184 | 201 | 201 |
| Thunderbird | stock | 3 | 453 | 446 | 428 | 240 | 292 | 320 |
| Thunderbird | shipped | 3 | 371 | 366 | 348 | 147 | 198 | 229 |
| Helium | stock | 3 | 427 | 405 | 405 | 297 | 308 | 318 |
| Helium | shipped | 3 | 325 | 302 | 302 | 195 | 204 | 211 |
| WebKitGTK MiniBrowser | stock | 3 | 778 | 774 | 804 | 224 | 386 | 495 |
| WebKitGTK MiniBrowser | shipped | 3 | 673 | 610 | 602 | 156 | 317 | 402 |
| Text Editor (GTK 4) | stock | 3 | 96 | 97 | 101 | 42 | 42 | 60 |
| Text Editor (GTK 4) | shipped | 3 | 78 | 78 | 83 | 22 | 22 | 40 |
| Wolkit Todos | stock | 3 | 77 | 77 | 78 | 34 | 33 | 44 |
| Wolkit Todos | shipped | 3 | 60 | 60 | 62 | 16 | 16 | 27 |

### RSS of the whole app, MiB, median of the runs (n in brackets)

| App | Mode | n | Start, 15 s | Front, idle 60 s | Front, typing 60 s | Behind 1 min | Behind 10 min | Back in front, 20 s |
|---|---|---|---|---|---|---|---|---|
| Discord | stock | 3 | 1134 | 994 | 1001 | 728 | 888 | 899 |
| Discord | shipped | 3 | 751 | 88 | 131 | 111 | 101 | 112 |
| Discord | shipped-nopageout | 3 | 822 | 696 | 121 | 111 | 108 | 119 |
| VS Code (Code - OSS) | stock | 3 | 1769 | 1570 | 1500 | 975 | 1105 | 1118 |
| VS Code (Code - OSS) | shipped | 3 | 1133 | 1088 | 1018 | 538 | 638 | 642 |
| Signal | stock | 3 | 970 | 871 | 871 | 669 | 687 | 688 |
| Signal | shipped | 3 | 721 | 622 | 622 | 397 | 415 | 416 |
| Thunderbird | stock | 3 | 708 | 686 | 669 | 479 | 532 | 560 |
| Thunderbird | shipped | 3 | 485 | 473 | 455 | 254 | 305 | 336 |
| Helium | stock | 3 | 1185 | 1149 | 1150 | 1042 | 1057 | 1068 |
| Helium | shipped | 3 | 676 | 648 | 649 | 542 | 551 | 559 |
| WebKitGTK MiniBrowser | stock | 3 | 1128 | 1121 | 1151 | 566 | 601 | 714 |
| WebKitGTK MiniBrowser | shipped | 3 | 855 | 868 | 898 | 387 | 468 | 570 |
| Text Editor (GTK 4) | stock | 3 | 155 | 155 | 159 | 100 | 100 | 118 |
| Text Editor (GTK 4) | shipped | 3 | 115 | 115 | 120 | 60 | 60 | 77 |
| Wolkit Todos | stock | 3 | 134 | 134 | 135 | 90 | 90 | 101 |
| Wolkit Todos | shipped | 3 | 95 | 95 | 96 | 50 | 50 | 62 |

### Where the memory is: PSS split into anonymous, file-backed and shared memory, MiB, median

| App | Mode | 15 s: anon | file | shmem | Behind 10 min: anon | file | shmem |
|---|---|---|---|---|---|---|---|
| Discord | stock | 396 | 222 | 22 | 209 | 221 | 8 |
| Discord | shipped | 347 | 121 | 6 | 14 | 13 | 0 |
| Discord | shipped-nopageout | 428 | 128 | 13 | 20 | 15 | 0 |
| VS Code (Code - OSS) | stock | 834 | 236 | 16 | 187 | 237 | 2 |
| VS Code (Code - OSS) | shipped | 705 | 99 | 17 | 197 | 118 | 3 |
| Signal | stock | 401 | 182 | 6 | 118 | 183 | 4 |
| Signal | shipped | 402 | 100 | 6 | 94 | 102 | 5 |
| Thunderbird | stock | 244 | 169 | 41 | 123 | 167 | 2 |
| Thunderbird | shipped | 243 | 88 | 40 | 110 | 87 | 1 |
| Helium | stock | 198 | 216 | 14 | 88 | 216 | 6 |
| Helium | shipped | 195 | 114 | 17 | 78 | 116 | 10 |
| WebKitGTK MiniBrowser | stock | 583 | 191 | 6 | 208 | 183 | 1 |
| WebKitGTK MiniBrowser | shipped | 563 | 104 | 6 | 208 | 106 | 1 |
| Text Editor (GTK 4) | stock | 61 | 34 | 1 | 7 | 34 | 1 |
| Text Editor (GTK 4) | shipped | 62 | 15 | 1 | 6 | 15 | 1 |
| Wolkit Todos | stock | 51 | 27 | 0 | 7 | 26 | 0 |
| Wolkit Todos | shipped | 51 | 10 | 0 | 6 | 10 | 0 |

### Behind 10 min: PSS, SwapPss (compressed in zram), the scope's memory.current, wakeups/s

| App | Mode | PSS | SwapPss | memory.current | Wakeups/s front idle | Wakeups/s behind (5-10 min) | CPU % of a core, front idle | CPU % of a core, behind (5-10 min) | Major faults/s, front idle | Major faults/s, behind | Page-outs counted |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Discord | stock | 438 | 86 | 313 | 220 | 225 | 3 | 1 | 0 | 1 | 0 |
| Discord | shipped | 28 | 258 | 100 | 565 | 426 | 13 | 18 | 132 | 1102 | 20 |
| Discord | shipped-nopageout | 35 | 251 | 100 | 188 | 413 | 2 | 14 | 0 | 964 | 0 |
| VS Code (Code - OSS) | stock | 423 | 365 | 414 | 67 | 38 | 2 | 1 | 0 | 62 | 0 |
| VS Code (Code - OSS) | shipped | 317 | 363 | 332 | 166 | 32 | 4 | 0 | 0 | 3 | 0 |
| Signal | stock | 306 | 184 | 111 | 63 | 130 | 2 | 0 | 0 | 2 | 0 |
| Signal | shipped | 201 | 208 | 14 | 63 | 135 | 2 | 0 | 0 | 2 | 0 |
| Thunderbird | stock | 292 | 86 | 179 | -2 | 7 | 0 | 0 | 0 | 13 | 0 |
| Thunderbird | shipped | 198 | 94 | 4 | 1 | 5 | 0 | 0 | 0 | 21 | 0 |
| Helium | stock | 308 | 88 | 54 | 1086 | 269 | 17 | 1 | 0 | 2 | 0 |
| Helium | shipped | 204 | 93 | 2 | 15 | 6 | 0 | 0 | 0 | 1 | 0 |
| WebKitGTK MiniBrowser | stock | 386 | 322 | 320 | 342 | 38 | 2 | 0 | 0 | 1 | 0 |
| WebKitGTK MiniBrowser | shipped | 317 | 225 | 178 | 463 | 44 | 6 | 0 | 0 | 0 | 0 |
| Text Editor (GTK 4) | stock | 42 | 59 | 1 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| Text Editor (GTK 4) | shipped | 22 | 61 | 0 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| Wolkit Todos | stock | 33 | 45 | 0 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| Wolkit Todos | shipped | 16 | 46 | 1 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |

### What the machine has left: MemAvailable of the whole 8 GiB guest, MiB, median (higher is better)

| App | Mode | Start, 15 s | Front, idle (last sample) | Behind 10 min |
|---|---|---|---|---|
| Discord | stock | 6388 | 6582 | 6652 |
| Discord | shipped | 6077 | 6478 | 6699 |
| Discord | shipped-nopageout | 6349 | 6589 | 6780 |
| VS Code (Code - OSS) | stock | 6039 | 6244 | 6580 |
| VS Code (Code - OSS) | shipped | 6163 | 6227 | 6510 |
| Signal | stock | 6494 | 6578 | 6686 |
| Signal | shipped | 6501 | 6586 | 6747 |
| Thunderbird | stock | 6633 | 6660 | 6759 |
| Thunderbird | shipped | 6627 | 6658 | 6779 |
| Helium | stock | 6672 | 6707 | 6779 |
| Helium | shipped | 6649 | 6697 | 6785 |
| WebKitGTK MiniBrowser | stock | 6289 | 6327 | 6636 |
| WebKitGTK MiniBrowser | shipped | 6325 | 6387 | 6619 |
| Text Editor (GTK 4) | stock | 6857 | 6865 | 6911 |
| Text Editor (GTK 4) | shipped | 6838 | 6840 | 6893 |
| Wolkit Todos | stock | 6873 | 6883 | 6917 |
| Wolkit Todos | shipped | 6832 | 6834 | 6868 |

### Coming back after 10 minutes (what a person feels)

| App | Mode | First visible change after the key, s | Last visible change, s | CPU in the first 4 s, ms | Major faults in 20 s | PSS at the return -> 20 s later |
|---|---|---|---|---|---|---|
| Discord | stock | 1.1 | 1.1 | 210 | 2836 | 439 -> 442 |
| Discord | shipped | 1.1 | 9.7 | 1540 | 29307 | 21 -> 35 |
| Discord | shipped-nopageout | 1.1 | 12.7 | 1830 | 35441 | 29 -> 42 |
| VS Code (Code - OSS) | stock | 0.9 | 2.2 | 240 | 4502 | 419 -> 439 |
| VS Code (Code - OSS) | shipped | 0.9 | 0.9 | 180 | 2364 | 310 -> 321 |
| Signal | stock | 0.9 | 1.6 | 80 | 274 | 306 -> 306 |
| Signal | shipped | 0.9 | 0.9 | 80 | 411 | 201 -> 201 |
| Thunderbird | stock | 1.1 | 1.1 | 200 | 1518 | 292 -> 320 |
| Thunderbird | shipped | 1.1 | 1.1 | 200 | 3240 | 198 -> 229 |
| Helium | stock | 0.9 | 0.9 | 780 | 1092 | 308 -> 318 |
| Helium | shipped | 0.9 | 1.6 | 80 | 1522 | 204 -> 211 |
| WebKitGTK MiniBrowser | stock | 1.0 | 2.6 | 2010 | 21880 | 386 -> 495 |
| WebKitGTK MiniBrowser | shipped | 1.2 | 2.5 | 2060 | 18344 | 317 -> 402 |
| Text Editor (GTK 4) | stock | 1.3 | 2.0 | 250 | 3864 | 42 -> 60 |
| Text Editor (GTK 4) | shipped | 1.0 | 1.8 | 230 | 4778 | 22 -> 40 |
| Wolkit Todos | stock | 1.1 | 4.5 | 180 | 1811 | 33 -> 44 |
| Wolkit Todos | shipped | 1.1 | 1.1 | 120 | 1861 | 16 -> 27 |

### Runs

| App | Mode | Runs ok | Runs failed | Window after the launcher, s (median) |
|---|---|---|---|---|
| Discord | stock | 3 | 0 | 21.2 |
| Discord | shipped | 3 | 0 | 21.2 |
| Discord | shipped-nopageout | 3 | 0 | 18.9 |
| VS Code (Code - OSS) | stock | 3 | 0 | 20.8 |
| VS Code (Code - OSS) | shipped | 3 | 0 | 19.2 |
| Signal | stock | 3 | 0 | 19.0 |
| Signal | shipped | 3 | 0 | 20.5 |
| Thunderbird | stock | 3 | 0 | 22.3 |
| Thunderbird | shipped | 3 | 0 | 20.5 |
| Helium | stock | 3 | 0 | 19.9 |
| Helium | shipped | 3 | 0 | 19.4 |
| WebKitGTK MiniBrowser | stock | 3 | 0 | 19.4 |
| WebKitGTK MiniBrowser | shipped | 3 | 0 | 19.4 |
| Text Editor (GTK 4) | stock | 3 | 0 | 19.2 |
| Text Editor (GTK 4) | shipped | 3 | 0 | 19.7 |
| Wolkit Todos | stock | 3 | 0 | 19.6 |
| Wolkit Todos | shipped | 3 | 0 | 19.6 |
