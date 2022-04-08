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

## shm-state

Work-In-Progress.

Additional utilities on-top of the share file descriptor concepts. In
particular, some semantic helper structures that help achieve hot-reloading,
state migration, or similar high-level goals.

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
