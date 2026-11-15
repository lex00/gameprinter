#!/usr/bin/env python3
"""disk.py DIR...: prints, for each DIR, two sizes in GB and the path:

  private  the bytes of DIR's files that no other file on the volume
           shares: APFS's own count (getattrlist's ATTR_CMNEXT_PRIVATESIZE).
           For a tree cloned from the work tree, this is what it costs on
           disk now: the files it rewrote, plus the old blocks it still holds
           after the work tree's builds replaced its own copies. Deleting
           DIR frees at least this much (more when several deleted trees
           share blocks only with each other).
  apparent the files' allocated size, as `du` counts it: shared blocks in
           full, so it overstates what a clone costs.

macOS only (APFS). A file whose private size can't be read is counted as
fully private.
"""

import ctypes
import ctypes.util
import os
import struct
import sys

ATTR_CMN_RETURNED_ATTRS = 0x80000000
ATTR_CMNEXT_PRIVATESIZE = 0x00000008
FSOPT_NOFOLLOW = 0x00000001
FSOPT_ATTR_CMN_EXTENDED = 0x00000020


class AttrList(ctypes.Structure):
    _fields_ = [
        ("bitmapcount", ctypes.c_ushort),
        ("reserved", ctypes.c_uint16),
        ("commonattr", ctypes.c_uint32),
        ("volattr", ctypes.c_uint32),
        ("dirattr", ctypes.c_uint32),
        ("fileattr", ctypes.c_uint32),
        ("forkattr", ctypes.c_uint32),
    ]


libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
ASK = AttrList(5, 0, ATTR_CMN_RETURNED_ATTRS, 0, 0, 0, ATTR_CMNEXT_PRIVATESIZE)
BUF = ctypes.create_string_buffer(64)


def private_size(path, allocated):
    """The file's unshared bytes; `allocated` when APFS doesn't say."""
    rc = libc.getattrlist(
        os.fsencode(path), ctypes.byref(ASK), BUF, len(BUF), FSOPT_NOFOLLOW | FSOPT_ATTR_CMN_EXTENDED
    )
    if rc != 0:
        return allocated
    # u32 length, five u32 of returned attributes, then the off_t.
    returned_ext = struct.unpack_from("<I", BUF.raw, 20)[0]
    if not returned_ext & ATTR_CMNEXT_PRIVATESIZE:
        return allocated
    return struct.unpack_from("<q", BUF.raw, 24)[0]


def measure(top):
    private = 0
    apparent = 0
    todo = [top]
    while todo:
        d = todo.pop()
        try:
            entries = list(os.scandir(d))
        except OSError:
            continue
        for e in entries:
            if e.is_dir(follow_symlinks=False):
                todo.append(e.path)
            elif e.is_file(follow_symlinks=False):
                try:
                    allocated = e.stat(follow_symlinks=False).st_blocks * 512
                except OSError:
                    continue
                apparent += allocated
                private += private_size(e.path, allocated)
    return private, apparent


def main():
    for top in sys.argv[1:]:
        private, apparent = measure(top)
        print(f"{private / 1e9:.2f}\t{apparent / 1e9:.2f}\t{top}")


if __name__ == "__main__":
    main()
