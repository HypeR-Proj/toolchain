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

static WORKER_DROPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct WorkerDrop;
impl Drop for WorkerDrop {
    fn drop(&mut self) {
        WORKER_DROPS.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
}
struct OnExit;
impl Drop for OnExit {
    fn drop(&mut self) {
        println!("HYPER_STD_TLS_DROP_OK");
    }
}
thread_local! {
    static WORKER_DROP: WorkerDrop = const { WorkerDrop };
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
    std::thread::Builder::new()
        .spawn(move || drop(child))
        .unwrap()
        .join()
        .unwrap();
    let shared = Arc::new((Mutex::new(0usize), Condvar::new()));
    let mut workers = Vec::new();
    for index in 0..4 {
        let shared = shared.clone();
        workers.push(std::thread::spawn(move || {
            WORKER_DROP.with(|_| ());
            COUNTER.with(|value| {
                assert_eq!(value.get(), 0);
                value.set(index + 1);
            });
            for _ in 0..200 {
                *shared.0.lock().unwrap() += 1;
            }
            shared.1.notify_all();
        }));
    }
    let mut progress = shared.0.lock().unwrap();
    while *progress < 800 {
        progress = shared.1.wait(progress).unwrap();
    }
    drop(progress);
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(WORKER_DROPS.load(std::sync::atomic::Ordering::Acquire), 4);
    for _ in 0..16 {
        let shared = shared.clone();
        drop(std::thread::spawn(move || {
            *shared.0.lock().unwrap() += 1;
            shared.1.notify_all();
        }));
    }
    let mut progress = shared.0.lock().unwrap();
    while *progress < 816 {
        progress = shared.1.wait(progress).unwrap();
    }
    drop(progress);
    let contended = RwLock::new(0usize);
    let barrier = std::sync::Barrier::new(5);
    std::thread::scope(|scope| {
        for index in 0..4 {
            let lock = &contended;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..200 {
                    if index % 2 == 0 {
                        *lock.write().unwrap() += 1;
                    } else {
                        assert!(*lock.read().unwrap() <= 400);
                    }
                }
            });
        }
        barrier.wait();
    });
    assert_eq!(*contended.read().unwrap(), 400);
    native_thread_stop();
    println!("HYPER_STD_THREADS_OK");
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

// Exercise the Native stop capability independently of std's cooperative join.
// The worker owns no heap/TLS resources; the caller retains its stack and word
// until the kernel's termination signal proves execution has detached.
fn native_thread_stop() {
    use hyper_os::handle::ThreadObject;
    use hyper_os::wait::{ObjectSignals, WaitItem};
    use std::sync::atomic::{AtomicU32, Ordering};
    extern "C" fn parked(argument: *const AtomicU32) -> ! {
        // SAFETY: native_thread_stop retains this word through TERMINATED.
        let word = unsafe { &*argument };
        word.store(1, Ordering::Release);
        loop {
            let _ = hyper_os::thread::atomic_wait(word, 1, u64::MAX);
        }
    }
    let word = AtomicU32::new(0);
    let mut stack = vec![0u8; 64 * 1024 + 16];
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !15;
    // SAFETY: a distinct retained stack and word are supplied; parked never
    // returns or accesses TLS. Both stay live until the terminal observation.
    let thread = unsafe {
        hyper_os::thread::create(
            parked as *const () as u64,
            top as u64,
            0,
            std::ptr::from_ref(&word) as u64,
        )
    }
    .unwrap();
    hyper_os::thread::start(thread.as_handle_ref()).unwrap();
    let start = Instant::now();
    while word.load(Ordering::Acquire) == 0 {
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    // SAFETY: the raw worker has no language-owned resources to abandon.
    unsafe { hyper_os::thread::request_stop(thread.as_handle_ref()) }.unwrap();
    let waits = [WaitItem::new(
        thread.as_handle_ref(),
        ObjectSignals::<ThreadObject>::TERMINATED,
    )];
    hyper_os::wait::wait_many(&waits, u64::MAX).unwrap();
    // SAFETY: TERMINATED was observed; the retained tombstone owns no execution.
    unsafe { hyper_os::thread::request_stop(thread.as_handle_ref()) }.unwrap();
    assert!(hyper_os::thread::start(thread.as_handle_ref()).is_err());
    drop(thread);
    drop(stack);
}
