#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Drop the cached pages of an app's files that no running process maps (posix_fadvise
DONTNEED), so the next start reads them again and they are charged to the scope it starts in.
Usage: evict.py DIR...  Prints the files and MiB asked."""
import os
import sys

n = size = 0
for top in sys.argv[1:]:
    for root, _dirs, files in os.walk(top):
        for f in files:
            p = os.path.join(root, f)
            try:
                fd = os.open(p, os.O_RDONLY | os.O_NOFOLLOW)
            except OSError:
                continue
            try:
                st = os.fstat(fd)
                os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
                n += 1
                size += st.st_size
            except OSError:
                pass
            finally:
                os.close(fd)
print(f'evict: {n} files, {size >> 20} MiB')
