// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

//! Exercise the real CRT/rtld handoff, including reclamation of old stack pages.
use hyper_os::handle::{HandleRef, VmarObject};
use hyper_os::memory::{PrivateMappingMode, PrivateMappingOptions, SnapshotVmo, WritableVmo};

pub fn run(root: HandleRef<'_, VmarObject>) {
    // The current non-ASLR kernel bootstrap contract: 128 KiB below 0x3f_f000,
    // with one 4 KiB guard at each end. This is not an application stack policy.
    const BASE: u64 = 0x3d_e000;
    const SIZE: u64 = 0x22000;
    let arguments: Vec<_> = std::env::args().collect();
    let environment: Vec<_> = std::env::vars().collect();
    let source = WritableVmo::create(SIZE).unwrap();
    let snapshot = SnapshotVmo::from_vmo(source.as_handle_ref()).unwrap();
    // SAFETY: startup has retired the bootstrap reservation before app entry.
    // EXACT reservation must fail rather than replace any still-live mapping.
    let mut reclaimed = unsafe {
        snapshot.map_private_at(
            root,
            BASE,
            PrivateMappingOptions {
                source_offset: 0,
                source_length: SIZE,
                data_offset: 0,
                size: SIZE,
                writable: true,
                mode: PrivateMappingMode::Eager,
            },
        )
    }
    .expect("bootstrap stack and guards must be released before main");
    reclaimed.as_mut_slice().unwrap().fill(0xcc);
    assert_eq!(std::env::args().collect::<Vec<_>>(), arguments);
    assert_eq!(std::env::vars().collect::<Vec<_>>(), environment);
    let local = 0u64;
    let address = &local as *const u64 as usize;
    let stack = hyper_os::thread::current_stack().unwrap();
    assert!((stack.base..stack.top).contains(&address));
    assert!(address < BASE as usize || address >= (BASE + SIZE) as usize);
    reclaimed.try_close().unwrap();
}
