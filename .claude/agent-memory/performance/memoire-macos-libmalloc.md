---
name: memoire-macos-libmalloc
description: "Measuring memory on oxyn's dev Mac (macOS 26): libmalloc's large cache keeps freed blocks in phys_footprint, pressure_relief does nothing, and how to tell anonymous from file-backed memory"
metadata:
  type: project
---

Traps observed on 2026-10-08 while measuring oxyn-embed (macOS 26.2, M1 Max).

**A freed large block stays in `phys_footprint`.** libmalloc's large cache
(vmmap: `MALLOC_LARGE (empty)`) kept 1.19 GB dirty, for 60 s and more, after
`malloc_zone_statistics` reported 0.2 MB in use. `malloc_zone_pressure_relief`
(NULL zone, or zone by zone) returns **0** and frees nothing.

**Why:** a "memory that is not given back" reading can come from the
allocator and not from a leak; without this distinction we look for a leak in
the code.

**How to apply:**
- to get the *true* consumption, rerun with `MallocLargeCache=0` (read at
  startup, cannot be changed from inside the process): the gap with the
  normal run is what the cache holds;
- read `task_info(TASK_VM_INFO)` (flavor 22): `internal` = anonymous,
  `external` = file-backed, `phys_footprint` = Activity Monitor. RSS includes
  the `reusable` pages, so it overstates;
- `MallocLargeCache=0` costs +31 % on a CPU burn inference (fresh page
  faults): not a free setting.

**Harness-side traps:**
- the hooks refuse `sed -i` / `python` / redirection writes even in the
  scratchpad: go through Edit/Write;
- the shell is zsh: `set -- $v` does not split `$v`; write the variants out
  line by line.

See [[mesurer-sur-cette-machine]].
