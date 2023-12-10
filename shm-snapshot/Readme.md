A memory-region that can be snapshot.

## Problem statement

The library implements some basic inter-process strategies for synchronizing
the data contained in a memory-mapped file. There is a host and a client,
residing over the same file descriptor passed from the environment, and pushed
through to the client by the host. The host's job is creating snapshots of the
data contained in the memory-mapped file and store such state persistently to
the disk, such that recent snapshots are automatically restored before
subsequent client executions.

The client is supposed to be able to be almost oblivious to actions of the
host, in particular the mechanism must be obstruction-free at least and better
yet should be wait-free (there's no added value in lock-free over
obstruction-free since running only the snapshot-making thread/process is
seldom useful but a permissible 'solution').

The main problem with regards to actual snapshots here is that no such
primitive exists for the arbitrarily large region of the whole file. Further,
the reads done by system calls such as `copy_file_range`, `write`, etc. can not
be reliably said to be sequenced with regards to anything done by either party.
There is no efficient implementation in which the promising to only read the
file 'forwards' / cycle through the file for instance, pages from a bulk copy
can land on disk in practically any order.

## General design

The host and client split the file into two areas: a set of memory pages
containing coordination structures and a tail of user data. Some mechanism then
guarantees that units from the coordinated data can be saved with atomic
semantics.

The client library generally provides a mechanism to sequence writes to the
coordination structure with writes to the user structure in such a way, that an
atomic copy of some coordination unit implies an atomic snapshot of a properly
client-accessed region of user memory. In this manner, arbitrary auxiliary data
can be attached to any snapshot. Note that during the time needed by the host
to persist a snapshot it necessary to freeze the corresponding coordination
unit. This need is extended to auxiliary data (the APIs help ensure this
property). You should generally prefer keeping auxiliary data in immutable data
structure so that snapshots-in-progress can share as much of the precious
memory resource that is the SHMFD, i.e. as much mutable portion of that memory
remains available.

## Details

### Queue strategy

A simple obstruction-free strategy, using a descriptor for data within a
continuous stream as a consistency mark. The host may be starved if its copy is
relatively slow compared to the clients creation of state.

This strategy does not require the host to map the data in a writable mode.

The client may optionally detect this condition by running its own routine
inspection of the successful snapshot file, and react accordingly to slowness.
This is not advised since it risks forward progress and/or waiting depending on
the file system, i.e. threatens the main benefit.

### Triple-buffer strategy

This strategy is not yet implemented. A CAS / hazard-pointer strategy where the
client creates and offers buffers with increments of the state. The host
returns those buffers when they're successfully contained in some persistent
snapshot.

This strategy require the host to map the data in a writable mode. The strategy
coordinates with the client.

It's not entirely clear to the author how efficiently this can be implemented.
On Linux we have futex waits but there may be unknown problems with regards to
obstruction freedom?

### Thoughts on an unimplemented strategy, remap based snapshots

Relies on a memory region being mapped in such a way that it can be remapped
with `MREMAP_DONTUNMAP` and having installed a `userfaultfd` to restore regions
via copy-on-write semantics.

There's a little detail here, it's not clear if hardware caches to the region
are already flushed. Indeed other processors than the one performing the
snapshot may have residual store instructions or receive results of stores
later than the apparent system call / inter-process communication after the
remap. It's unclear if it is necessary to establish additional happens-before
relationships with all 'subsequent' writes after the snapshot to ensure absence
of such pending changes. For this reason, the library must keep track of its
writers and await a completion flag from them regardless.
