// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

mod cow;
mod files;
mod processes;
mod relay;
mod stack_workload;
mod virtual_serial;
mod wait_sets;

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
    #[arg(long, hide = true)]
    uncalibrated_clock: bool,
    #[arg(long)]
    panic: bool,
    #[arg(long, hide = true)]
    child: Option<String>,
    #[arg(long, hide = true, num_args = 0..=1, default_missing_value = "all", value_parser = ["all", "inspect", "memory", "process"])]
    stack_workload: Option<String>,
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
    if args.uncalibrated_clock {
        let before = std::time::SystemTime::now();
        let uptime = before.duration_since(std::time::UNIX_EPOCH).unwrap();
        assert!(uptime < Duration::from_secs(86400));
        std::thread::spawn(|| std::thread::sleep(Duration::from_millis(20)))
            .join()
            .unwrap();
        let after = std::time::SystemTime::now();
        assert!(after > before);
        std::fs::write("/clock-fallback-test", b"clock").unwrap();
        let created = std::fs::metadata("/clock-fallback-test")
            .unwrap()
            .created()
            .unwrap();
        assert!(created >= before && created <= std::time::SystemTime::now());
        std::fs::remove_file("/clock-fallback-test").unwrap();
        println!("HYPER_STD_UNCALIBRATED_CLOCK_OK");
        return;
    }
    if let Some(mode) = args.child.as_deref() {
        processes::child(mode);
        return;
    }
    if let Some(stage) = args.stack_workload.as_deref() {
        let result = stack_workload::run(stage);
        assert!(result.is_ok(), "stack workload failed: {result:?}");
        return;
    }
    if args.panic {
        panic!("HYPER_STD_EXPECTED_PANIC");
    }
    let mut startup = hyper_rt::process::startup().unwrap();
    assert!(hyper_rt::process::startup().is_err());
    assert!(hyper_rt::process::stdin().is_ok());
    assert!(hyper_rt::process::stdout().is_ok());
    assert!(hyper_rt::process::stderr().is_ok());
    let cow_result = startup
        .borrow(hyper_os::startup::ROOT_VMAR)
        .map_err(|error| format!("{error:?}"))
        .and_then(cow::run);
    assert!(cow_result.is_ok(), "COW regression: {cow_result:?}");
    virtual_serial::run(&startup.take(hyper_os::startup::ROOT_VMAR).unwrap());
    files::scoped_startup_root(&mut startup);
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
    let relay_result = relay::verify();
    assert!(
        relay_result.is_ok(),
        "byte relay regression: {relay_result:?}"
    );
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
    detached_native_exit();
    check_stack_growth();
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(check_stack_growth)
        .unwrap()
        .join()
        .unwrap();
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
    floating_point_threads();
    println!("HYPER_STD_THREADS_OK");
    assert_eq!(Arc::strong_count(&captured), 1);
    files::run();
    wait_sets::run();
    processes::run();
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
    extern "C" fn cpu_bound(argument: *const AtomicU32) -> ! {
        // SAFETY: native_thread_stop retains this word through TERMINATED.
        let word = unsafe { &*argument };
        word.store(1, Ordering::Release);
        loop {
            std::hint::spin_loop();
        }
    }
    let word = AtomicU32::new(0);
    let mut stack = vec![0u8; 64 * 1024 + 16];
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !15;
    // SAFETY: a distinct retained stack and word are supplied; cpu_bound never
    // returns or accesses TLS. Both stay live until the terminal observation.
    let thread = unsafe {
        hyper_os::thread::create(
            cpu_bound as *const () as u64,
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

// Runtime operands prevent constant folding; distinct per-thread accumulators
// cross both immediate and blocking syscalls while competing for execution.
fn floating_point_threads() {
    let workers: Vec<_> = (1..=8)
        .map(|index| {
            std::thread::spawn(move || {
                COUNTER.with(|value| value.set(index));
                let step = std::hint::black_box(index as f64 * 0.125);
                let mut value = std::hint::black_box(0.0f64);
                for iteration in 0..4096 {
                    value += step;
                    if iteration % 257 == 0 {
                        std::thread::sleep(Duration::from_micros(1));
                    } else if iteration % 31 == 0 {
                        std::thread::yield_now();
                    }
                    COUNTER.with(|local| assert_eq!(local.get(), index));
                    std::hint::black_box(value);
                }
                assert_eq!(value, index as f64 * 512.0);
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    println!("HYPER_STD_FP_THREADS_OK");
}

fn check_stack_growth() {
    let before = hyper_os::thread::current_stack().unwrap();
    let local = 0x1234_u64;
    let address = &local as *const u64 as usize;
    assert!((before.base..before.top).contains(&address));
    hyper_os::thread::grow_current_stack(before.size + 64 * 1024).unwrap();
    let after = hyper_os::thread::current_stack().unwrap();
    assert_eq!(after.top, before.top);
    assert_eq!(after.capacity, before.capacity);
    assert_eq!(after.size, before.size + 64 * 1024);
    // SAFETY: newly mapped stack bytes below the old extent are disjoint from
    // every current frame. Touch both ends to verify backing is accessible.
    unsafe {
        (after.base as *mut u8).write_volatile(42);
        ((before.base - 1) as *mut u8).write_volatile(24);
    }
    assert_eq!(local, 0x1234);
    assert!(hyper_os::thread::grow_current_stack(after.capacity + 4096).is_err());
    assert_eq!(hyper_os::thread::current_stack().unwrap(), after);
}

// Bypass the ordinary worker return path. Repeated large VA reservations prove
// the detached reaper observes kernel termination, not a trampoline-only flag.
fn detached_native_exit() {
    unsafe extern "C" {
        fn hyper_runtime_thread_spawn_with_stack(
            size: usize,
            capacity: usize,
            entry: extern "C" fn(*mut core::ffi::c_void),
            argument: *mut core::ffi::c_void,
            token: *mut usize,
        ) -> i64;
        fn hyper_runtime_thread_release(token: usize);
        fn hyper_runtime_thread_detach();
        fn hyper_thread_exit(status: i64) -> !;
    }
    extern "C" fn exit_directly(_: *mut core::ffi::c_void) {
        // SAFETY: this fixture owns no Rust/TLS values; detach releases its
        // runtime TLS allocation before exiting without returning to worker.
        unsafe {
            hyper_runtime_thread_detach();
            hyper_thread_exit(0);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    for _ in 0..96 {
        loop {
            let mut token = 0;
            // SAFETY: fixed entry takes no borrowed data; ownership of each
            // successful token is immediately transferred to the reaper.
            let status = unsafe {
                hyper_runtime_thread_spawn_with_stack(
                    64 * 1024,
                    8 * 1024 * 1024,
                    exit_directly,
                    core::ptr::null_mut(),
                    &mut token,
                )
            };
            if status == 0 {
                // SAFETY: this successful token has exactly one caller owner.
                unsafe { hyper_runtime_thread_release(token) };
                break;
            }
            assert_eq!(
                status,
                hyper_os::Status::NO_MEMORY.as_raw(),
                "unexpected stack admission error"
            );
            assert!(
                Instant::now() < deadline,
                "detached stack reservations leaked"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
