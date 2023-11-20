A memory-region that can be snapshot.

## Details

Relies on a memory region being mapped in such a way that it can be remapped
with `MREMAP_DONTUNMAP` and having installed a `userfaultfd` to restore regions
via copy-on-write semantics.

There's a little detail here, that it's not clear if hardware caches to the
region are already flushed. Other processors than the one performing the
snapshot may have residual store instructions. It's unclear if it is necessary
to establish additional happens-before relationships with all 'subsequent'
writes after the snapshot to ensure absence of such pending changes. For this
reason, the library must keep track of its writers and await a completion flag
from them regardless.
