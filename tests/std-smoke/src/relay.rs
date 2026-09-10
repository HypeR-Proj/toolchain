// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

//! Exercise stale readiness, bounded backpressure, and EOF on real channels.

use hyper_os::channel::{ByteRelay, create_pair};
use hyper_os::handle::ByteChannelObject;
use hyper_os::wait::{ObjectSignals, WaitItem, wait_many};
use hyper_os::{Error, Status};

pub fn verify() -> hyper_os::Result<()> {
    let (producer, source) = create_pair()?;
    let (destination, consumer) = create_pair()?;
    let mut storage = [0; 32];
    let mut received = [0; 32];
    let mut relay = ByteRelay::new(
        source.as_byte_channel(),
        destination.as_byte_channel(),
        &mut storage,
    );

    producer.as_byte_channel().try_send(b"old")?;
    wait_many(
        &[WaitItem::new(
            source.as_handle_ref(),
            ObjectSignals::<ByteChannelObject>::READABLE,
        )],
        hyper_os::DEADLINE_INFINITE,
    )?;
    // Another read invalidates the notification. A multiplexing relay must
    // return to its caller instead of sleeping solely on this empty channel.
    assert_eq!(source.as_byte_channel().try_receive(&mut received)?, 3);
    assert!(!relay.poll()?);

    let mut queued = 0;
    for _ in 0..64 {
        match destination.as_byte_channel().try_send(b"full") {
            Ok(()) => queued += 1,
            Err(Error::Status(Status::WOULD_BLOCK)) => break,
            Err(error) => return Err(error),
        }
    }
    assert!(queued > 0 && queued < 64);
    producer.as_byte_channel().try_send(b"pending")?;
    assert!(relay.poll()?);
    assert!(!relay.poll()?);
    assert!(relay.has_pending());

    let (reverse_producer, reverse_source) = create_pair()?;
    let (reverse_destination, reverse_consumer) = create_pair()?;
    let mut reverse_storage = [0; 32];
    let mut reverse = ByteRelay::new(
        reverse_source.as_byte_channel(),
        reverse_destination.as_byte_channel(),
        &mut reverse_storage,
    );
    reverse_producer.as_byte_channel().try_send(b"progress")?;
    assert!(reverse.poll()?);
    assert!(reverse.poll()?);
    assert_eq!(
        reverse_consumer
            .as_byte_channel()
            .try_receive(&mut received)?,
        8
    );
    assert_eq!(&received[..8], b"progress");
    assert!(!relay.poll()?);

    for _ in 0..queued {
        assert_eq!(consumer.as_byte_channel().try_receive(&mut received)?, 4);
        assert_eq!(&received[..4], b"full");
    }
    assert!(relay.poll()?);
    assert_eq!(consumer.as_byte_channel().try_receive(&mut received)?, 7);
    assert_eq!(&received[..7], b"pending");
    assert!(matches!(
        consumer.as_byte_channel().try_receive(&mut received),
        Err(Error::Status(Status::WOULD_BLOCK))
    ));

    producer.as_byte_channel().try_send(b"")?;
    drop(producer);
    assert!(relay.poll()?);
    assert!(relay.poll()?);
    assert_eq!(consumer.as_byte_channel().try_receive(&mut received)?, 0);
    assert!(relay.poll()?);
    assert!(relay.is_finished());
    assert!(!relay.has_pending());
    assert!(relay.wait_item().is_none());
    assert!(!relay.poll()?);
    Ok(())
}
