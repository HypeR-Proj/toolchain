// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

use hyper_os::startup::Startup;
use hyper_rt::ExitCode;

fn application_main(startup: Startup<'_>) -> ExitCode {
    if hyper_os::require_core_abi().is_err() || startup.console().is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

hyper_rt::entry!(application_main);
