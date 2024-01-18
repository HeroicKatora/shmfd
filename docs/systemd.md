This repository plays nice with systemd's File Descriptor store. Keep in mind,
that only the main process (as per PID) can interact with the notify socket.
Hence, we provide a wrapper binary which initialized the more robust interior
setup. In fact, the main `shm-fd` executable can be a convenient way to
initialize and persist a new shared memory when the notify socket is detected
with its environment variable `$NOTIFY_SOCKET` but also works standalone when
no such socket is available.

**These examples add systemd unit files to your user configuration, execute
them if you know what you're doing**. The bash scripts work from within the
docs folder or from the repository root.

### Oneshot

In the oneshot service case, a binary working on the memory file is launched to
terminate after some fixed unit of work was performed. It the performs the next
increment in the next run. This is modeled by a unit file which can be
restarted by appropriate conditions, such as timer units or socket activation,
etc. The state of the shared memory is preserved without disk involvement by
persisting it with the `shm-fd` wrapper.

The [`systemd.oneshot.template`](./systemd.oneshot.template) file is a basic
unit skeleton with the necessary configuration.

```bash
export SHMFD="$(pwd)"
[ -f systemd.md ] && export SHMFD="$(realpath ..)"

cargo build --release -p shm-fd -p primes

envsubst > ~/.config/systemd/user/shmfd-primes.service \
    < "$SHMFD/docs/systemd.oneshot.template"

systemctl --user daemon-reload
systemctl --user start shmfd-primes

# To display output
_invocation=`systemctl --user show --value -p InvocationID shmfd-primes`
journalctl --user _SYSTEMD_INVOCATION_ID=$_invocation

# Now restart the service, which does not unload its file descriptor store
systemctl --user restart shmfd-primes
echo "Service restarted, output of second run follows"

# To display output
_invocation=`systemctl --user show --value -p InvocationID shmfd-primes`
journalctl --user _SYSTEMD_INVOCATION_ID=$_invocation
```

### Exec unit

In an executable service, the shared memory file is used to keep continuous
snapshots for recovery during abnormal termination (i.e. crash, loss-of-power).
In contrast to the above, the unit is considered inactive when stopped and
reloads its state from disk in case of fresh starts or recovery.

```bash
export SHMFD="$(pwd)"
[ -f systemd.md ] && export SHMFD="$(realpath ..)"

cargo build --release \
    -p shm-snapshot --features=shm-restore --bin shm-restore \
    -p primes-snapshot --bin primes-snapshot

envsubst > ~/.config/systemd/user/shmfd-primes-snapshot.service \
    < "$SHMFD/docs/systemd.service.template"

truncate -s 100M "$SHMFD/target/fprimes-snapshot"
systemctl --user daemon-reload
systemctl --user start shmfd-primes-snapshot

# To display output
journalctl --user --unit shmfd-primes-snapshot -n 20 --no-pager

# Now SIGKILL the service, trigger abnormal termination
systemctl --user kill -s SIGKILL shmfd-primes-snapshot.service

# To display output, show recovery.
journalctl --user --unit shmfd-primes-snapshot -n 20 --no-pager --follow

systemctl --user stop shmfd-primes-snapshot
```
