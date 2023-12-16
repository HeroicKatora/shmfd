use shmfd_test_validate::Env;
use shmfd_test_executables::SHM_PRIMES_SNAPSHOT;

use std::process::Command;

fn main() {}

#[test]
fn primes_snapshot() {
    let env = Env::new();
    env.shared_fd({
        let mut cmd = Command::new(SHM_PRIMES_SNAPSHOT);
        cmd.args(["1", "10000"]);
        cmd
    }).success();
}
