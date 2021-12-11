# shm-state

A variant of `systemfd` opening a shared-memory file which is utilized to
preserve state across runs of a program, coordinated by a watcher creating the
original file.

## Try it out

There is a simple prime sieve example (like, Eratosthenes simplicity). It
generates at most 1000 values each run, filling up a share memory file of 1MB.
When no more values are to be generated then it prints the full table.

It's easily executed in a loop with `watch`.

```
cargo build --release -p shm-fd -p primes
./target/release/shm-fd watch -n 0.1 ./target/release/primes
```
