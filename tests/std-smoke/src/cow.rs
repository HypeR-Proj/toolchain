// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

use hyper_os::handle::{HandleRef, VmarObject};
use hyper_os::memory::{
    PAGE_SIZE, PrivateMappingMode, PrivateMappingOptions, SnapshotVmo, WritableVmo,
};
use std::io::Read;
use std::sync::atomic::{AtomicU32, Ordering};

fn show(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

pub fn run(root: HandleRef<'_, VmarObject>) -> Result<(), String> {
    const BASE: u64 = 0xd300_0000;
    let page = PAGE_SIZE as usize;
    let source = WritableVmo::create(3 * PAGE_SIZE)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    source
        .write_all_at(0, &vec![0x11; 3 * page])
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let snapshot = SnapshotVmo::from_vmo(source.as_handle_ref())
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    source
        .write_all_at(0, &vec![0x22; 3 * page])
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let options = PrivateMappingOptions {
        source_offset: 0,
        source_length: 3 * PAGE_SIZE,
        data_offset: 0,
        size: 3 * PAGE_SIZE,
        writable: true,
        mode: PrivateMappingMode::CopyOnWrite,
    };
    // SAFETY: these disjoint fixed ranges are reserved for this acceptance test;
    // the root belongs to this process and only each returned owner touches it.
    let mut first = unsafe { snapshot.map_private_at(root, BASE, options) }
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    // SAFETY: same ownership proof for a distinct, nonoverlapping range.
    let second = unsafe { snapshot.map_private_at(root, BASE + 0x10000, options) }
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    // SAFETY: same ownership proof; eager allocation has identical isolation.
    let mut eager = unsafe {
        snapshot.map_private_at(
            root,
            BASE + 0x20000,
            PrivateMappingOptions {
                mode: PrivateMappingMode::Eager,
                ..options
            },
        )
    }
    .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    assert!(first.as_slice().iter().all(|byte| *byte == 0x11));
    first
        .as_mut_slice()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?[page] = 0x33;
    eager
        .as_mut_slice()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?[page] = 0x44;
    assert_eq!(first.as_slice()[page], 0x33);
    assert_eq!(second.as_slice()[page], 0x11);
    assert_eq!(eager.as_slice()[page], 0x44);
    let mut captured = [0; 1];
    snapshot
        .read_exact_at(PAGE_SIZE, &mut captured)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    assert_eq!(captured, [0x11]);

    // Exercise copyout into an untouched COW page: a direct VMO write here
    // would corrupt the second view and the immutable source.
    let path = "/cow-copyout";
    std::fs::write(path, b"private-copyout")
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    file.read_exact(
        &mut first
            .as_mut_slice()
            .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?[..15],
    )
    .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    assert_eq!(&first.as_slice()[..15], b"private-copyout");
    assert!(second.as_slice()[..15].iter().all(|byte| *byte == 0x11));
    std::fs::remove_file(path).map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;

    // A partial source page must not disclose bytes in its prefix/tail. BSS
    // is zero even when the next source file page contains nonzero bytes.
    let boundary = PrivateMappingOptions {
        source_length: PAGE_SIZE + 19,
        data_offset: 17,
        ..options
    };
    // SAFETY: a fourth disjoint range has the same unique-owner contract.
    let mut partial = unsafe { snapshot.map_private_at(root, BASE + 0x30000, boundary) }
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    assert!(partial.as_slice()[..17].iter().all(|byte| *byte == 0));
    assert!(
        partial.as_slice()[17..page + 36]
            .iter()
            .all(|byte| *byte == 0x11)
    );
    assert!(
        partial.as_slice()[page + 36..]
            .iter()
            .all(|byte| *byte == 0)
    );
    partial
        .as_mut_slice()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?[2 * page] = 0x55;
    assert_eq!(second.as_slice()[2 * page], 0x11);

    // These atomics begin on untouched shared pages. The Native wait/wake pin
    // must materialize private backing before retaining its physical address.
    let zero = WritableVmo::create(PAGE_SIZE)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let zero_snapshot = SnapshotVmo::from_vmo(zero.as_handle_ref())
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    // SAFETY: this fifth range is private to the test for the owner's lifetime.
    let mut atomics = unsafe {
        zero_snapshot.map_private_at(
            root,
            BASE + 0x40000,
            PrivateMappingOptions {
                source_length: PAGE_SIZE,
                size: PAGE_SIZE,
                ..options
            },
        )
    }
    .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let address = atomics
        .as_mut_slice()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?
        .as_mut_ptr() as usize;
    // SAFETY: the page is aligned, zero initialized, writable and retained;
    // it is accessed atomically until all scoped workers have joined.
    let word = unsafe { AtomicU32::from_ptr(address as *mut u32) };
    assert!(matches!(
        hyper_os::thread::atomic_wait(word, 0, 0),
        Err(hyper_os::Error::Status(hyper_os::Status::TIMED_OUT))
    ));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(move || {
                for _ in 0..1000 {
                    word.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(word.load(Ordering::Acquire), 4000);
    hyper_os::thread::atomic_wake(word, u32::MAX)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let mut original = [1; 4];
    zero_snapshot
        .read_exact_at(0, &mut original)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    assert_eq!(original, [0; 4]);

    // Independent writers fault concurrently into the same initially shared
    // page. A losing preparation must retry without losing the winner's writes.
    // SAFETY: this distinct range is owned until every scoped worker joins.
    let mut concurrent = unsafe {
        zero_snapshot.map_private_at(
            root,
            BASE + 0x50000,
            PrivateMappingOptions {
                source_length: PAGE_SIZE,
                size: PAGE_SIZE,
                ..options
            },
        )
    }
    .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let address = concurrent
        .as_mut_slice()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?
        .as_mut_ptr() as usize;
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for index in 0..4 {
            let start = &start;
            scope.spawn(move || {
                // SAFETY: each worker uses a distinct aligned word, with only
                // atomic accesses; the enclosing scope retains the whole page.
                let word = unsafe { AtomicU32::from_ptr((address + index * 4) as *mut u32) };
                start.wait();
                for _ in 0..1000 {
                    word.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    for word in concurrent.as_slice()[..16].chunks_exact(4) {
        assert_eq!(word, 1000_u32.to_ne_bytes());
    }
    concurrent
        .try_close()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    atomics
        .try_close()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    partial
        .try_close()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    eager
        .try_close()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    second
        .try_close()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    first
        .try_close()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    // Exact VA reuse proves mapping/VMAR teardown completed, including views
    // which never faulted and still retained shared source pages at unmap.
    for _ in 0..8 {
        // SAFETY: the previous owner was dropped before each reuse.
        let mut mapping = unsafe { snapshot.map_private_at(root, BASE, options) }
            .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
        mapping
            .as_mut_slice()
            .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?[0] = 0x66;
        mapping
            .try_close()
            .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    }
    capability_copyout(root, BASE + 0x60000)?;
    println!("HYPER_COW_OK");
    Ok(())
}

fn capability_copyout(root: HandleRef<'_, VmarObject>, address: u64) -> Result<(), String> {
    use hyper_abi as abi;
    use hyper_os::capability_channel::{CapabilityChannel, CapabilityDisposition};
    use hyper_os::handle::{CapabilityChannelObject, OwnedHandle, Rights, RightsOffer, VmoObject};
    use std::num::NonZeroU64;
    use std::time::Duration;

    let page = PAGE_SIZE as usize;
    let rights = Rights::READ.union(Rights::INSPECT);
    let slot = abi::HyperNativeCapabilityReceiveSlot {
        handle: 0,
        rights: abi::HYPER_NATIVE_RIGHT_READ | abi::HYPER_NATIVE_RIGHT_INSPECT,
        expected_kind: abi::HYPER_NATIVE_OBJECT_VMO,
        flags: 0,
    };
    let mut template = vec![0u8; 5 * page];
    // SAFETY: this repr(C) record has no padding and every field is initialized.
    let slot_bytes = unsafe {
        std::slice::from_raw_parts((&raw const slot).cast::<u8>(), std::mem::size_of_val(&slot))
    };
    template[page..page + slot_bytes.len()].copy_from_slice(slot_bytes);
    let source = WritableVmo::create(5 * PAGE_SIZE)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    source
        .write_all_at(0, &template)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let snapshot = SnapshotVmo::from_vmo(source.as_handle_ref())
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    // SAFETY: this disjoint range is owned until receive, sender and inspection
    // finish. No output page is touched by userspace before its syscall writes.
    let mut outputs = unsafe {
        snapshot.map_private_at(
            root,
            address,
            PrivateMappingOptions {
                source_offset: 0,
                source_length: 5 * PAGE_SIZE,
                data_offset: 0,
                size: 5 * PAGE_SIZE,
                writable: true,
                mode: PrivateMappingMode::CopyOnWrite,
            },
        )
    }
    .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let output = outputs
        .as_mut_slice()
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?
        .as_mut_ptr();
    // SAFETY: successful create transfers two exclusive endpoint handles.
    let pair = unsafe { hyper_sys::capability_channel_create() };
    raw_status(pair.status)?;
    let send_raw = NonZeroU64::new(pair.value0).ok_or("missing send endpoint")?;
    let receive_raw = NonZeroU64::new(pair.value1).ok_or("missing receive endpoint")?;
    // SAFETY: each successful create output is adopted exactly once.
    let sender = CapabilityChannel::from_handle(unsafe {
        OwnedHandle::<CapabilityChannelObject>::from_raw_owned(send_raw)
    });
    // SAFETY: this guard owns the distinct receive endpoint throughout raw use.
    let _receiver = unsafe { OwnedHandle::<CapabilityChannelObject>::from_raw_owned(receive_raw) };
    let raw = source.into_handle().into_raw();
    // SAFETY: into_raw transferred this sole owner; adoption immediately restores it.
    let source_owner = unsafe { OwnedHandle::<VmoObject>::from_raw_owned(raw) };
    let deadline = hyper_os::time::deadline_after(Duration::from_secs(10))
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    let sender_page = output.expose_provenance() + 4 * page;
    let result = std::thread::scope(|scope| -> Result<_, String> {
        let worker = scope.spawn(|| -> Result<(), String> {
            hyper_os::wait::wait_many(
                &[hyper_os::wait::WaitItem::new(
                    sender.as_handle_ref(),
                    hyper_os::wait::ObjectSignals::PEER_RECEIVING,
                )],
                deadline.as_raw(),
            )
            .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
            // SAFETY: this fifth aligned page is retained by outputs; only this
            // worker touches its word. PEER_RECEIVING proves receive holds both
            // copyout reservations while this unrelated write resolves COW.
            unsafe { AtomicU32::from_ptr(std::ptr::with_exposed_provenance_mut(sender_page)) }
                .store(7, Ordering::Release);
            let mut dispositions = [CapabilityDisposition::duplicate(
                source_owner.as_handle_ref(),
                RightsOffer::Exact(rights),
            )
            .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?];
            let stop = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                match sender.try_send(b"cow-rendezvous", &mut dispositions) {
                    Ok(()) => return Ok(()),
                    Err(hyper_os::Error::Status(hyper_os::Status::WOULD_BLOCK)) => {
                        if std::time::Instant::now() >= stop {
                            return Err("COW sender timed out".into());
                        }
                        std::thread::yield_now();
                    }
                    Err(error) => return Err(show(error)),
                }
            }
        });
        // SAFETY: two distinct aligned pages are writable, retained, and initially
        // shared. The receive-slot request was initialized in the source template,
        // so constructing its pointer has not resolved either COW fault in advance.
        let received = unsafe {
            hyper_sys::capability_channel_receive(
                receive_raw.get(),
                deadline.as_raw(),
                output,
                page,
                output.add(page).cast(),
                1,
            )
        };
        worker.join().map_err(|_| "COW sender panicked")??;
        raw_status(received.status)?;
        Ok(received)
    })?;
    assert_eq!((result.value0, result.value1), (14, 1));
    assert_eq!(&outputs.as_slice()[..14], b"cow-rendezvous");
    assert_eq!(
        &outputs.as_slice()[4 * page..4 * page + 4],
        &7u32.to_ne_bytes()
    );
    // SAFETY: OK initialized exactly one aligned slot and transferred its handle.
    let received_slot = unsafe {
        output
            .add(page)
            .cast::<abi::HyperNativeCapabilityReceiveSlot>()
            .read()
    };
    let received_raw = NonZeroU64::new(received_slot.handle).ok_or("missing received VMO")?;
    // SAFETY: adopt the single output handle exactly once, retaining it for info calls.
    let _received_owner = unsafe { OwnedHandle::<VmoObject>::from_raw_owned(received_raw) };
    // SAFETY: these separate aligned pages are still untouched COW mappings and
    // remain live throughout each raw info copyout; the received handle has INSPECT.
    let (handle_result, object_result) = unsafe {
        (
            hyper_sys::handle_get_info(received_raw.get(), output.add(2 * page).cast()),
            hyper_sys::object_get_basic_info(received_raw.get(), output.add(3 * page).cast()),
        )
    };
    raw_status(handle_result.status)?;
    raw_status(object_result.status)?;
    // SAFETY: both successful calls initialized their complete aligned records.
    let (handle, object) = unsafe {
        (
            output
                .add(2 * page)
                .cast::<abi::HyperNativeHandleInfo>()
                .read(),
            output
                .add(3 * page)
                .cast::<abi::HyperNativeObjectBasicInfo>()
                .read(),
        )
    };
    assert_eq!(handle.object_kind, abi::HYPER_NATIVE_OBJECT_VMO);
    assert_eq!(handle.rights, slot.rights);
    assert_eq!(object.object_kind, handle.object_kind);
    assert_ne!(object.koid, 0);
    let mut unchanged = vec![0u8; 5 * page];
    snapshot
        .read_exact_at(0, &mut unchanged)
        .map_err(|error| format!("COW line {}: {}", line!(), show(error)))?;
    assert_eq!(unchanged, template);
    Ok(())
}

fn raw_status(status: hyper_abi::HyperNativeStatus) -> Result<(), String> {
    if status == hyper_abi::HYPER_NATIVE_STATUS_OK {
        Ok(())
    } else {
        Err(format!("COW copyout syscall failed: {status}"))
    }
}
