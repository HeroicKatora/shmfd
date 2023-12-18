use core::sync::atomic::{AtomicU64, Ordering};

use shm_fd::SharedFd;
use shm_snapshot::{ConfigureFile, File, PreparedTransaction, Writer, Snapshot};

fn main() {
    let Some(fd) = (unsafe { SharedFd::from_env() }) else {
        panic!("No shared memory state found");
    };

    let (writer, mut state) = restore_from(fd);
    let interval = std::env::args()
        .nth(1)
        .map_or(1000, |num| {
            num.parse().unwrap()
        });

    let limit = std::env::args_os()
        .nth(2)
        .and_then(|os| -> Option<u64> {
            let arg = os.as_os_str().to_str()?;
            arg.parse().ok()
        })
        .unwrap_or(u64::MAX);

    const CHUNK: u64 = 100000;
    let mut chunk = 0..state.prime_last;
    let chunks = core::iter::from_fn(move || {
        let ret_chunk = chunk.clone();
        let new_end = chunk.end + CHUNK;
        chunk.start = chunk.end;
        chunk.end = new_end;
        Some(ret_chunk)
    });

    let mut writer = writer;
    for chunk in chunks.skip(1) {
        if state.prime_total > limit {
            break;
        }

        let put = &chunk.end.to_be_bytes();
        let (_, new_state) = writer.write_with(put, |tx| run_main_routine(tx, chunk)).unwrap();
        state = new_state;

        std::thread::sleep(std::time::Duration::from_millis(interval));
    }
}

struct State {
    prime_total: u64,
    prime_last: u64,
}

/// A very simple prime sieve..
fn run_main_routine(mut tx: PreparedTransaction<'_>, num_range: core::ops::Range<u64>)
    -> Option<State>
{
    let values = tx.tail();

    if values[0].load(Ordering::Relaxed) == 0 {
        values[0].store(2, Ordering::Relaxed);
        values[1].store(3, Ordering::Relaxed);
    }

    let (pos, _num) = values
        .iter()
        .take_while(|num| num.load(Ordering::Relaxed) != 0)
        .enumerate()
        .last()
        .expect("at least one prime");

    // The first number above the new prime to check.
    let mut num = 0;
    let mut pos = pos + 1;

    if pos >= values.len() {
        println!("No more primes to fill");
        eprintln!("{:?}", &values[..]);
        return None;
    }

    for candidate in num_range {
        // Check divisibility for all prior primes.
        if !check_prime(candidate, &values[..pos as usize]) {
            continue;
        }

        // Found a new prime.
        values[pos as usize].store(candidate, Ordering::Relaxed);
        num += 1;
        pos += 1;
    }

    let post_place: u64 = pos as u64;
    tx.replace(&post_place.to_be_bytes());
    eprintln!("generated {} more primes, total {}", num, post_place);
    Some(State {
        prime_total: post_place,
        prime_last: values[pos as usize].load(Ordering::Relaxed),
    })
}

fn check_prime(num: u64, primes: &[AtomicU64]) -> bool {
    let bound = upper_int_sqrt(num);
    for p in primes {
        if p.load(Ordering::Relaxed) > bound {
            break;
        }

        if (num % p.load(Ordering::Relaxed)) == 0 {
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

fn restore_from(fd: SharedFd) -> (Writer, State) {
    struct ExtendWith<F>(F);

    impl<F> Extend<Snapshot> for ExtendWith<F>
        where F: FnMut(Snapshot)
    {
        fn extend<T: IntoIterator<Item = Snapshot>>(&mut self, iter: T) {
            for item in iter {
                (self.0)(item);
            }
        }
    }

    let file = fd.into_file().expect("opening shared fd failed");
    let _ = file.set_len(100_000_000u64);
    let mut mapping = File::new(file).unwrap();
    let mut config = ConfigureFile::default();

    let mut latest_snapshot = None;
    if let Some(mapping) = mapping.recover(&mut config) {
        let mut restore_state = ExtendWith(|snapshot: Snapshot| {
            latest_snapshot = std::cmp::max_by_key(
                    latest_snapshot, Some(snapshot),
                    |x: &Option<Snapshot>| x.map(|v| v.offset)
                );
        });

        mapping.valid(&mut restore_state);
    }

    config.or_insert_with(|cfg| {
        cfg.entries = 0x100;
        cfg.data = 0x800;
    });

    let writer = mapping.configure(&config);
    let prime_total = if let Some(latest_snapshot) = latest_snapshot {
        let mut buffer = [0; 8];
        writer.read(&latest_snapshot, &mut buffer);
        u64::from_be_bytes(buffer)
    } else {
        0
    };

    eprintln!("Recovering {prime_total} existing primes");
    let (retain, scratch) = writer.tail().split_at(prime_total as usize);

    let prime_last = if let Some(prime_last) = retain.last() {
        for item in scratch {
            item.store(0, Ordering::Relaxed);
        }

        prime_last.load(Ordering::Relaxed)
    } else {
        2
    };

    let state = State {
        prime_last,
        prime_total,
    };

    (writer, state)
}
