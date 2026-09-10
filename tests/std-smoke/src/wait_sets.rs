// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

use hyper_os::handle::{ByteChannelObject, Rights};
use hyper_os::wait::{ObjectSignals, WaitSet};
use hyper_os::{Error, Status};

fn deadline() -> u64 {
    hyper_os::time::monotonic_now()
        .unwrap()
        .checked_deadline_after(std::time::Duration::from_secs(5))
        .unwrap()
        .as_raw()
}

pub fn run() {
    let (reader, writer) = hyper_os::channel::create_pair().unwrap();
    let owner = WaitSet::new(1).unwrap().into_handle();
    let consumer = WaitSet::from_handle(owner.duplicate(Rights::WAIT).unwrap());
    let set = WaitSet::from_handle(owner);
    let readable = ObjectSignals::<ByteChannelObject>::READABLE;
    assert!(matches!(
        consumer.add(reader.as_handle_ref(), readable),
        Err(Error::Status(Status::ACCESS_DENIED))
    ));
    assert!(matches!(
        set.add(
            set.as_handle_ref(),
            ObjectSignals::<hyper_os::handle::WaitSetObject>::READABLE
        ),
        Err(Error::Status(Status::INVALID_ARGUMENT))
    ));
    let id = set.add(reader.as_handle_ref(), readable).unwrap();
    assert!(matches!(
        set.add(reader.as_handle_ref(), readable),
        Err(Error::Status(Status::RESOURCE_LIMIT))
    ));
    assert!(matches!(set.wait(0), Err(Error::Status(Status::TIMED_OUT))));
    writer.as_byte_channel().send(b"abcd").unwrap();
    assert!(matches!(set.rearm(id), Err(Error::Status(Status::BUSY))));
    let event = consumer.wait(deadline()).unwrap();
    assert!(event.sequence > 0);
    assert_eq!(event.registration, id);
    assert!(readable.is_present_in(event.signals));
    assert!(matches!(set.wait(0), Err(Error::Status(Status::TIMED_OUT))));
    set.rearm(id).unwrap(); // The unread level is still asserted.
    assert_eq!(set.wait(deadline()).unwrap().registration, id);
    let mut bytes = [0; 4];
    assert_eq!(reader.as_byte_channel().receive(&mut bytes).unwrap(), 4);
    set.rearm(id).unwrap();
    writer.as_byte_channel().send(b"abcd").unwrap();
    set.remove(id).unwrap();
    assert!(matches!(set.wait(0), Err(Error::Status(Status::TIMED_OUT))));
    assert!(matches!(
        set.rearm(id),
        Err(Error::Status(Status::NOT_FOUND))
    ));
    reader.as_byte_channel().receive(&mut bytes).unwrap();
    let closed = ObjectSignals::<ByteChannelObject>::PEER_CLOSED;
    let id = set.add(reader.as_handle_ref(), closed).unwrap();
    let copy = writer.duplicate(Rights::WRITE.union(Rights::WAIT)).unwrap();
    drop(writer);
    assert!(matches!(set.wait(0), Err(Error::Status(Status::TIMED_OUT))));
    drop(copy);
    assert_eq!(set.wait(deadline()).unwrap().registration, id);
    drop(consumer);
    drop(set);
    drop(reader);

    // Persistent sets intentionally exceed the per-call wait_many limit.
    let set = WaitSet::new(80).unwrap();
    let mut endpoints = Vec::new();
    let mut expected = std::collections::HashSet::new();
    for _ in 0..80 {
        let (reader, writer) = hyper_os::channel::create_pair().unwrap();
        expected.insert(set.add(reader.as_handle_ref(), readable).unwrap());
        writer.as_byte_channel().send(b"ready").unwrap();
        endpoints.push((reader, writer));
    }
    for _ in 0..80 {
        assert!(expected.remove(&set.wait(deadline()).unwrap().registration));
    }
    assert!(expected.is_empty());
    drop(set); // Detach all live source registrations before releasing endpoints.
    drop(endpoints);

    let (reader, writer) = hyper_os::channel::create_pair().unwrap();
    let set = WaitSet::new(1).unwrap();
    let id = set.add(reader.as_handle_ref(), readable).unwrap();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(5));
        writer.as_byte_channel().send(b"wake").unwrap();
    });
    assert_eq!(set.wait(deadline()).unwrap().registration, id);
    worker.join().unwrap();
    drop(reader); // Operation pins must not become active endpoint authority.
    drop(set);
    println!("HYPER_NATIVE_WAIT_SET_OK");
}
