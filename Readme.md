# shm-fd

A variant of `systemfd` opening a shared-memory file which is utilized to
preserve state across runs of a program, coordinated by a watcher creating the
original file and duplicating a file descriptor to its child.

## Library hierarchy

- [`shm-fd`] is mostly a library/binary component to setup and consume a file
  descriptor to a shared file, specifying the default transport by which it is
  passed (i.e. the environment variable and how to interpret the setting).
- [`shm-snapshot`] is an intermediate layer that provides persistent snapshot
  backups of the state represented in the shared memory file. Its
  `shm-restore` binary implements the client side of the `shm-snapshot`.
- [`shm-state`] is a work-in-progress high-level library on top of the
  restore/atomic journal mechanisms which aims to provide efficient data
  structures in which a binary might keep state. These data structure should
  provide all access characteristics of (immutable) memory data structures with
  the snapshot persistence of the journaling.

[`shm-fd`]: ./shm-fd/Readme.md
[`shm-snapshot`]: ./shm-snapshot/Readme.md
[`shm-state`]: ./shm-state/Readme.md

## Try out the file mechanism

There is a simple prime sieve example (like, Eratosthenes simplicity). It
generates at most 1000 values each execution, filling up a share memory file of
1MB. When no more values are to be generated then it prints the full table.

It's easily executed in a loop with `watch`.

```bash
cargo build --release -p shm-fd -p primes
./target/release/shm-fd watch -n 0.1 ./target/release/primes
```

Here the `shm-fd` program opens the file and keeps it available, and watch
loops the inner program repeatedly on that state.

## Try out snapshot & restore

The snapshot&restore mechanism is also demonstrated with a prime sieve program.
It generates a throughput limited stream of values while running. The wrapper
ensures that data is reloaded and persisted at start and end of the process.

```bash
cargo build --release -p shm-fd \
    -p shm-snapshot --features=shm-restore --bin shm-restore \
    -p primes-snapshot

./target/release/shm-fd \
    ./target/release/shm-restore ./target/prime-snapshot \
    ./target/release/primes-snapshot
```

See [`shm-snapshot`] for more information on continuous consistent snapshots.

## Repository structure

Multiple packages are grouped in one workspace here, all related to the
functionality. However, the tests are in their own separate workspace. This
avoids dependencies populating too much. Integration or unit tests are not
feasible for the core invariants since they require coordination among
processes.

## shm-state

Work-In-Progress.

Additional utilities on-top of the share file descriptor concepts. In
particular, some semantic helper structures that help achieve hot-reloading,
state migration, or similar high-level goals.
