// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

//! Real Native dispatch and grant checks; the acceptance kernel uses P=20 ms.

use hyper_os::handle::ThreadObject;
use hyper_os::scheduling::{self, Parameters, Policy, Role};
use hyper_os::wait::{ObjectSignals, WaitItem};
use hyper_os::{Error, Rights, Status};

pub fn run() -> Result<(), String> {
    // Demotion is intentionally irreversible for ordinary apps. Keep it inside
    // this worker so the rest of the std acceptance suite retains its priority.
    std::thread::Builder::new()
        .name("std-scheduling".into())
        .spawn(verify_current)
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| String::from("scheduling worker panicked"))??;
    println!("HYPER_STD_SCHEDULING_OK");
    Ok(())
}

fn verify_current() -> Result<(), String> {
    let original = scheduling::current().map_err(show)?;
    let Policy::Fair { weight, .. } = original.policy else {
        return Err(String::from("ordinary app did not receive Fair scheduling"));
    };
    if weight < 3 {
        return Err(String::from("acceptance app needs default weight >= 3"));
    }
    let lower = with_weight(original, weight - 1)?;
    scheduling::set_current(lower).map_err(show)?;
    equal(
        scheduling::current().map_err(show)?,
        lower,
        "current demotion",
    )?;

    denied(scheduling::set_current(original), "restore priority")?;
    equal(
        scheduling::current().map_err(show)?,
        lower,
        "failed priority update",
    )?;
    let changed_role = Parameters {
        role: if lower.role == Role::Priority {
            Role::Ordinary
        } else {
            Role::Priority
        },
        ..lower
    };
    denied(scheduling::set_current(changed_role), "change role")?;
    equal(
        scheduling::current().map_err(show)?,
        lower,
        "failed class update",
    )?;
    denied(
        scheduling::set_current(Parameters {
            policy: Policy::Budget {
                budget_ns: 10_000_000,
                period_ns: 20_000_000,
                burst_ns: 5_000_000,
            },
            ..lower
        }),
        "assign budget",
    )?;
    equal(
        scheduling::current().map_err(show)?,
        lower,
        "failed budget update",
    )?;

    std::thread::Builder::new()
        .name("std-scheduling-child".into())
        .spawn(move || -> Result<(), String> {
            equal(
                scheduling::current().map_err(show)?,
                lower,
                "child inheritance",
            )?;
            denied(scheduling::set_current(original), "child elevation")?;
            equal(
                scheduling::current().map_err(show)?,
                lower,
                "rejected child elevation",
            )
        })
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| String::from("scheduling child panicked"))??;
    verify_thread_handle(lower)
}

fn verify_thread_handle(inherited: Parameters) -> Result<(), String> {
    extern "C" fn dormant_entry(_: u64) -> ! {
        loop {
            core::hint::spin_loop();
        }
    }
    let mut stack = vec![0_u8; 64 * 1024 + 16];
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !15;
    // SAFETY: this is a distinct retained stack; dormant_entry never touches
    // TLS. The thread is never started, and is stopped and joined below before
    // its stack is released.
    let thread =
        unsafe { hyper_os::thread::create(dormant_entry as *const () as u64, top as u64, 0, 0) }
            .map_err(show)?;
    let result = (|| {
        if thread.info().map_err(show)?.rights.bits() & scheduling::RIGHTS.bits()
            != Rights::SCHED_LOWER.bits()
        {
            return Err(String::from("new Thread has unexpected scheduling rights"));
        }
        equal(
            scheduling::thread(thread.as_handle_ref()).map_err(show)?,
            inherited,
            "Thread inheritance",
        )?;
        let readonly = thread.duplicate(Rights::INSPECT).map_err(show)?;
        equal(
            scheduling::thread(readonly.as_handle_ref()).map_err(show)?,
            inherited,
            "INSPECT query",
        )?;
        let Policy::Fair { weight, .. } = inherited.policy else {
            return Err(String::from("inherited policy changed"));
        };
        let lower = with_weight(inherited, weight - 1)?;
        denied(
            scheduling::set_thread(readonly.as_handle_ref(), lower),
            "INSPECT modification",
        )?;
        equal(
            scheduling::thread(thread.as_handle_ref()).map_err(show)?,
            inherited,
            "rejected handle update",
        )?;
        scheduling::set_thread(thread.as_handle_ref(), lower).map_err(show)?;
        equal(
            scheduling::thread(thread.as_handle_ref()).map_err(show)?,
            lower,
            "LOWER handle update",
        )?;
        denied(
            scheduling::set_thread(thread.as_handle_ref(), inherited),
            "Thread priority restoration",
        )?;
        equal(
            scheduling::thread(thread.as_handle_ref()).map_err(show)?,
            lower,
            "rejected Thread restoration",
        )
    })();
    // SAFETY: the dormant thread has never executed and owns no language state.
    unsafe { hyper_os::thread::request_stop(thread.as_handle_ref()) }.map_err(show)?;
    hyper_os::wait::wait_many(
        &[WaitItem::new(
            thread.as_handle_ref(),
            ObjectSignals::<ThreadObject>::TERMINATED,
        )],
        u64::MAX,
    )
    .map_err(show)?;
    result
}

fn with_weight(mut parameters: Parameters, value: u16) -> Result<Parameters, String> {
    let Policy::Fair { ref mut weight, .. } = parameters.policy else {
        return Err(String::from("Fair parameters required"));
    };
    *weight = value;
    Ok(parameters)
}

fn equal(actual: Parameters, expected: Parameters, operation: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{operation}: {actual:?} != {expected:?}"))
    }
}

fn denied(result: hyper_os::Result<()>, operation: &str) -> Result<(), String> {
    if result == Err(Error::Status(Status::ACCESS_DENIED)) {
        Ok(())
    } else {
        Err(format!(
            "{operation}: expected ACCESS_DENIED, received {result:?}"
        ))
    }
}

fn show(error: Error) -> String {
    format!("{error:?}")
}
