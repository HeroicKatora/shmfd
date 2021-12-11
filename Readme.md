# shm-state

A variant of `systemfd` opening a shared-memory file which is utilized to
preserve state across runs of a program, coordinated by a watcher creating the
original file.
