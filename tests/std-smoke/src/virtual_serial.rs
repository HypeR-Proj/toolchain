// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

use hyper_os::handle::{VirtualSerialObject, VmarObject};
use hyper_os::wait::{ObjectSignals, WaitSet};
use hyper_os::{Error, Status};

pub fn run(root: &hyper_os::OwnedHandle<VmarObject>) {
    let serial = hyper_os::virtual_serial::create().unwrap();
    let memory =
        hyper_os::memory::WritableVmo::create(hyper_os::virtual_serial::BUFFER_BYTES).unwrap();
    let mut output = hyper_os::virtual_serial::Output::register(
        &serial,
        root.as_handle_ref(),
        0xd100_0000,
        memory,
    )
    .unwrap();
    let mut bytes = [0; 32];
    assert_eq!(output.try_read(&mut bytes).unwrap(), 0);
    assert_eq!(output.lost_bytes(), 0);
    let waits = WaitSet::new(1).unwrap();
    let id = waits
        .add(
            serial.as_handle_ref(),
            ObjectSignals::<VirtualSerialObject>::READABLE
                .union(ObjectSignals::<VirtualSerialObject>::PEER_CLOSED),
        )
        .unwrap();
    drop(serial); // Output retains its own consumer authority.
    assert!(matches!(
        waits.wait(0),
        Err(Error::Status(Status::TIMED_OUT))
    ));
    drop(output); // Subscription lifetime must not retain active authority.
    let event = waits.wait(0).unwrap();
    assert_eq!(event.registration, id);
    assert!(ObjectSignals::<VirtualSerialObject>::PEER_CLOSED.is_present_in(event.signals));
    assert!(!ObjectSignals::<VirtualSerialObject>::READABLE.is_present_in(event.signals));
    waits.remove(id).unwrap();
    println!("HYPER_VIRTUAL_SERIAL_WAIT_OK");
}
