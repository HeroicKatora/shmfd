use memmap2::MmapMut;
use shm_fd::SharedFd;
use std::os::unix::io::AsRawFd;

fn main() {
    let file;
    let mut mapping;
    let memory;

    if let Some(fd) = unsafe { SharedFd::from_env() } {
        file = fd.into_file().expect("opening shared fd failed");
        let _ = file.set_len(100_000_000u64);
        mapping = unsafe { MmapMut::map_mut(file.as_raw_fd()) }.expect("memmap failed");
        memory = &mut mapping[..];
    } else {
        panic!("No shared memory state found");
    }

    let values = bytemuck::cast_slice_mut(memory);
    run_main_routine(values);
}

/// A very simple prime sieve..
fn run_main_routine(values: &mut [u64]) {
    const CHUNK: usize = 100000;

    if values[0] == 0 {
        values[0] = 2;
        values[1] = 3;
    }

    let (pos, num) = values
        .iter()
        .take_while(|num| **num != 0)
        .enumerate()
        .last()
        .expect("at least one prime");
    // The position to insert a new prime.
    let pos = pos + 1;
    let end = (pos + CHUNK).min(values.len());
    // The first number above the new prime to check.
    let mut num = num + 1;

    if pos >= end {
        println!("No more primes to fill");
        eprintln!("{:?}", &values[..]);
        return;
    }

    'slot: for slot in pos..end {
        for candidate in num.. {
            // Check divisibility for all prior primes.
            if !check_prime(candidate, &values[..slot]) {
                continue;
            }

            // Found a new prime.
            values[slot] = candidate;
            num = candidate + 1;

            continue 'slot;
        }

        // No more numbers to check..
        break;
    }

    eprintln!("generated {} more primes, total {}", CHUNK, end);
}

fn check_prime(num: u64, primes: &[u64]) -> bool {
    let bound = upper_int_sqrt(num);
    for &p in primes {
        if p > bound {
            break;
        }

        if (num % p) == 0 {
            return false;
        }
    }

    true
}

fn upper_int_sqrt(num: u64) -> u64 {
    let mut l = 0;
    let mut r = num + 1;

    // Loop invariant: l < sqrt(num) <= r
    // Termination: r - l is strictly decreasing.
    while l != r - 1 {
        // Avoids overflow because l < r
        // Also note that m < r for this reason
        // And l < m as r - l > 1
        let m = l + (r - l) / 2;
        if m * m < num {
            // preserves l < r
            l = m;
        } else {
            r = m;
        }
    }

    r
}
