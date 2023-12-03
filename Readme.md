# shm-fd

A variant of `systemfd` opening a shared-memory file which is utilized to
preserve state across runs of a program, coordinated by a watcher creating the
original file and duplicating a file descriptor to its child.

## Try it out

There is a simple prime sieve example (like, Eratosthenes simplicity). It
generates at most 1000 values each execution, filling up a share memory file of
1MB. When no more values are to be generated then it prints the full table.

It's easily executed in a loop with `watch`.

```bash
cargo build --release -p shm-fd -p primes
./target/release/shm-fd watch -n 0.1 ./target/release/primes
```

## Library hierarchy

- `shm-fd` is mostly a library/binary component to setup and consume a file
  descriptor to a shared file, specifying the default transport by which it is
  passed (i.e. the environment variable and how to interpret the setting).
- `shm-restore` is an intermediate layer that provides persistent snapshot
  backups of the state represented in the shared memory file.
- `shm-snapshot` implements the client side of the `shm-restore`. WIP: maybe we
  should merge this with `shm-restore` since it provides two components of the
  same concept and they must share at least layout of their mechanisms?
- `shm-state` is a high-level library on top of the restore/atomic journal
  mechanisms which aims to provide efficient data structures in which a binary
  might keep state. These data structure should provide all access
  characteristics of (immutable) memory data structures with the snapshot
  persistence of the journaling.

## shm-restore

Lightweight Checkpoint and Restore for shm-fd programs. Writes `shm-fd` data to
a backing file and restores such data before running the actual program. In
particular catches `SIGTERM`. This allows it to persist data transparently
across (system) restarts.

Example:

```bash
cargo build --release -p shm-fd -p shm-restore -p primes
./target/release/shm-fd ./target/release/shm-restore ./target/prime-snapshot ./target/release/primes
./target/release/shm-fd ./target/release/shm-restore ./target/prime-snapshot ./target/release/primes
hexdump ./target/prime-shapshot
```

Or, to test the Strg+C (SIGINT) way of interrupting:

```bash
cargo build --release -p shm-fd -p shm-restore -p primes
./target/release/shm-fd ./target/release/shm-restore ./target/prime-snapshot watch -n 0.1 ./target/release/primes
^C
hexdump ./target/prime-shapshot
```

## shm-state

Work-In-Progress.

Additional utilities on-top of the share file descriptor concepts. In
particular, some semantic helper structures that help achieve hot-reloading,
state migration, or similar high-level goals.
