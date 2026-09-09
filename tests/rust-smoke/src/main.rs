// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{boxed::Box, format, vec::Vec};
use hyper_os::startup::Startup;
use hyper_rt::ExitCode;

fn application_main(startup: Startup<'_>) -> ExitCode {
    if hyper_os::require_core_abi().is_err()
        || startup.console().is_err()
        || hyper_service::stdio::STANDARD_OUTPUT.as_raw() == 0
    {
        return ExitCode::FAILURE;
    }
    let mut values = Vec::new();
    for value in 0..4096u64 {
        values.push(value);
    }
    let text = format!("values={}", values.len());
    let joined = [b"heap ".as_slice(), b"ready".as_slice()].concat();
    #[repr(align(8192))]
    struct Aligned(u64);
    let aligned = Box::new(Aligned(42));
    if text != "values=4096"
        || joined != b"heap ready"
        || values.get(4095) != Some(&4095)
        || aligned.0 != 42
        || (&*aligned as *const Aligned as usize) % 8192 != 0
    {
        return ExitCode::FAILURE;
    }
    if values.try_reserve(512 * 1024 * 1024).is_ok() || values.get(4095) != Some(&4095) {
        return ExitCode::FAILURE;
    }
    drop((values, text, joined, aligned));
    match startup
        .console()
        .and_then(|console| console.write_all(b"HYPER_RUST_HEAP_OK\n"))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

hyper_rt::entry!(application_main);
