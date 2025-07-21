# rts-alloc

[![Rust CI](https://github.com/apfitzge/rts-alloc/actions/workflows/ci.yml/badge.svg)](https://github.com/apfitzge/rts-alloc/actions/workflows/ci.yml)

`rts-alloc` provides an shared memory `Allocator` for use in sharing
allocations between processes.
The allocator is a lock-free slab allocator.
It is meant for allocating small objects quickly and is NOT meant for
use as a general purpose allocator.

There is a maximum number of "workers" (thread/process) that may use the
allocator at any given time.
Each worker must be assigned a unique `id` when creating or joining the
allocator space.

When a worker allocates memory, it must have an owned slab within the shared
memory space.
Each slab is divided into "slots" of memory for given size classes
(see `size_classes.rs`).
When a worker allocates memory, it will first try to allocate in slabs it
already owns.
If there are no suitable slots in owned slabs, it will try to take a slab
from the global pool.
Since all workers pull and push to this global pool, it is a source of
contention - and should be avoided whenever possible.
Once popped from the global pool, the slab is owned by the worker and is
assigned to a specific size class.


When a worker frees allocations it created itself, the newly freed slot of
memory within the slab is immediately returned and made available for reuse by
the same worker.
When a worker frees allocations created by another worker, the slot is added
to a free list which will only become available for reuse by the worker that
owns the corresponding slab cleans its remote free lists.
There is some contention when cleaning these free lists, in finding the initial
head of the list, so this should be done as infrequently as possible.
Of course, the worker also does not want to clean so infrequently that it runs
out of available memory space.

When the owning worker has freed all slots within an owned slab, it must
return the slab! [^1]

[^1]: or suffer my curse
