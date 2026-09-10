// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

// Execute the actual pinned upstream lock algorithms and our futex adapter
// with simultaneous host threads. No copied or simplified lock implementation.
#![feature(generic_atomic)]
#![allow(dead_code)]

pub use std::{hint, sync, time};
mod sys {
    mod ffi {
        unsafe extern "C" {
            pub fn __hyper_std_clock() -> u64;
            pub fn hyper_runtime_wait_u32(address: *const u32, expected: u32, deadline: u64)
            -> i32;
            pub fn hyper_runtime_wake_u32(address: *const u32, count: u32) -> u32;
        }
    }
    pub mod futex {
        include!(concat!(
            env!("HYPER_STD_OVERLAY"),
            "/std/src/sys/pal/hyper/futex.rs"
        ));
    }
    pub mod mutex {
        include!(concat!(
            env!("HYPER_RUST_LIBRARY"),
            "/std/src/sys/sync/mutex/futex.rs"
        ));
    }
    pub mod rwlock {
        include!(concat!(
            env!("HYPER_RUST_LIBRARY"),
            "/std/src/sys/sync/rwlock/futex.rs"
        ));
    }
    #[allow(unsafe_op_in_unsafe_fn)] // Matches std::sys for the included upstream file.
    pub mod condvar {
        include!(concat!(
            env!("HYPER_RUST_LIBRARY"),
            "/std/src/sys/sync/condvar/futex.rs"
        ));
    }
    pub mod sync {
        pub use super::mutex::Mutex;
    }
}

use std::cell::UnsafeCell;
use std::sync::Arc;

struct Shared<T> {
    lock: T,
    value: UnsafeCell<usize>,
}
// SAFETY: tests access the value only while holding the actual tested lock.
unsafe impl<T: Sync> Sync for Shared<T> {}

#[test]
fn mutex_serializes_concurrent_writers() {
    let shared = Arc::new(Shared {
        lock: sys::mutex::Mutex::new(),
        value: UnsafeCell::new(0),
    });
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let shared = &shared;
            scope.spawn(move || {
                for _ in 0..5000 {
                    shared.lock.lock();
                    unsafe {
                        *shared.value.get() += 1;
                        shared.lock.unlock();
                    }
                }
            });
        }
    });
    assert_eq!(unsafe { *shared.value.get() }, 20000);
}

#[test]
fn rwlock_wakes_readers_and_writers() {
    let shared = Shared {
        lock: sys::rwlock::RwLock::new(),
        value: UnsafeCell::new(0),
    };
    std::thread::scope(|scope| {
        for index in 0..6 {
            let shared = &shared;
            scope.spawn(move || {
                for _ in 0..3000 {
                    if index % 2 == 0 {
                        shared.lock.write();
                        unsafe {
                            *shared.value.get() += 1;
                            shared.lock.write_unlock();
                        }
                    } else {
                        shared.lock.read();
                        unsafe {
                            assert!(*shared.value.get() <= 9000);
                            shared.lock.read_unlock();
                        }
                    }
                }
            });
        }
    });
    assert_eq!(unsafe { *shared.value.get() }, 9000);
}

#[test]
fn condvar_has_no_lost_notification() {
    let shared = Shared {
        lock: sys::mutex::Mutex::new(),
        value: UnsafeCell::new(0),
    };
    let cv = sys::condvar::Condvar::new();
    let ready = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let shared = &shared;
        let cv = &cv;
        let ready = &ready;
        scope.spawn(move || {
            shared.lock.lock();
            ready.wait();
            while unsafe { *shared.value.get() } == 0 {
                unsafe { cv.wait(&shared.lock) };
            }
            unsafe { shared.lock.unlock() };
        });
        // The consumer holds the mutex until wait begins, so the producer
        // cannot satisfy the predicate before the wait path is exercised.
        ready.wait();
        shared.lock.lock();
        unsafe {
            *shared.value.get() = 1;
            shared.lock.unlock();
        }
        cv.notify_one();
    });
}
