// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(version, about = "HypeR standard library acceptance probe")]
struct Args {
    #[arg(long, default_value = "world", env = "HYPER_STD_NAME")]
    name: String,
    #[arg(long)]
    read_input: bool,
    #[arg(long)]
    panic: bool,
}

struct OnExit;
impl Drop for OnExit {
    fn drop(&mut self) {
        println!("HYPER_STD_TLS_DROP_OK");
    }
}
thread_local! {
    static COUNTER: Cell<usize> = const { Cell::new(0) };
    static ON_EXIT: OnExit = const { OnExit };
}

fn main() {
    let args = Args::parse();
    if args.panic {
        panic!("HYPER_STD_EXPECTED_PANIC");
    }
    let startup = hyper_rt::process::startup().unwrap();
    assert!(hyper_rt::process::startup().is_err());
    assert!(hyper_rt::process::stdin().is_ok());
    assert!(hyper_rt::process::stdout().is_ok());
    assert!(hyper_rt::process::stderr().is_ok());
    drop(startup); // The remaining I/O and TLS destructor still need these handles.
    COUNTER.with(|value| {
        value.set(42);
        assert_eq!(value.get(), 42);
    });
    ON_EXIT.with(|_| {});
    assert_eq!(std::env::consts::OS, "hyper");
    let mut values = HashMap::new();
    values.insert("name", args.name.clone());
    assert_eq!(values.get("name"), Some(&args.name));
    let once = OnceLock::new();
    assert_eq!(once.get_or_init(|| 7), &7);
    let lock = RwLock::new(1);
    *lock.write().unwrap() = 2;
    assert_eq!(*lock.read().unwrap(), 2);
    let mutex = Mutex::new(1);
    let guard = mutex.lock().unwrap();
    assert!(mutex.try_lock().is_err());
    let now = Instant::now();
    let (_guard, timeout) = Condvar::new()
        .wait_timeout(guard, Duration::from_millis(1))
        .unwrap();
    assert!(timeout.timed_out());
    assert!(now.elapsed() >= Duration::from_millis(1));
    std::thread::current().unpark();
    std::thread::park();
    std::thread::park_timeout(Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(1));
    let captured = Arc::new(42);
    let child = Arc::clone(&captured);
    assert_eq!(
        std::thread::Builder::new()
            .spawn(move || drop(child))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(Arc::strong_count(&captured), 1);
    assert_eq!(
        std::fs::File::open("/missing").unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(
        std::net::TcpStream::connect("127.0.0.1:80")
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    if args.read_input {
        println!("HYPER_STD_INPUT_READY");
        // Deliberately smaller than the test's channel message: no truncation.
        let mut input = [0; 3];
        io::stdin().read_exact(&mut input).unwrap();
        assert_eq!(&input, b"abc");
        io::stdin().read_exact(&mut input).unwrap();
        assert_eq!(&input, b"def");
    }
    io::stderr()
        .lock()
        .write_all(b"HYPER_STD_STDERR_OK\n")
        .unwrap();
    println!("HYPER_STD_OK hello {}", args.name);
}
