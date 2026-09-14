// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

//! Bounded identical userspace workload for kernel stack/performance comparison.
//! The test launcher must supply TASK_INSPECTOR in addition to ordinary std
//! process startup authority. No time threshold is meaningful under QEMU.

use hyper_os::handle::{HandleRef, VmarObject};
use hyper_os::inspect::{ScanCursor, TaskInspector};
use hyper_os::memory::{
    PAGE_SIZE, PrivateMappingMode, PrivateMappingOptions, SnapshotVmo, WritableVmo,
};
use std::process::{Command, Stdio};
use std::time::Instant;

fn show(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

pub fn run(stage: &str) -> Result<(), String> {
    let mut startup = hyper_rt::process::startup().map_err(show)?;
    let inspector = TaskInspector::from_handle(
        startup
            .take(hyper_os::startup::TASK_INSPECTOR)
            .map_err(show)?,
    );
    let root = startup.borrow(hyper_os::startup::ROOT_VMAR).map_err(show)?;
    if matches!(stage, "all" | "inspect") {
        let started = Instant::now();
        let mut pages = 0usize;
        let mut entries = 0usize;
        for _ in 0..32 {
            let mut cursor = Some(ScanCursor::START);
            let mut scan_pages = 0usize;
            while let Some(position) = cursor {
                let page = inspector.scan_threads(position).map_err(show)?;
                pages += 1;
                entries += page.entries().count();
                scan_pages += 1;
                if scan_pages > 4096 {
                    return Err("thread scan did not terminate within its bound".into());
                }
                cursor = page.next();
            }
        }
        if entries == 0 {
            return Err("thread inspector returned no visible threads".into());
        }
        println!(
            "HYPER_STACK_WORKLOAD stage=inspect operations=32 pages={pages} entries={entries} elapsed_ns={} success=1",
            started.elapsed().as_nanos()
        );
    }
    if matches!(stage, "all" | "memory") {
        let started = Instant::now();
        for iteration in 0..32 {
            memory_iteration(root, iteration)?;
        }
        println!(
            "HYPER_STACK_WORKLOAD stage=memory operations=32 snapshot_pages=256 mappings=64 cow_faults=32 elapsed_ns={} success=1",
            started.elapsed().as_nanos()
        );
    }
    if matches!(stage, "all" | "process") {
        let executable = std::env::args().next().ok_or("missing executable path")?;
        let started = Instant::now();
        for _ in 0..4 {
            let status = Command::new(&executable)
                .args(["--child", "null"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(show)?;
            if !status.success() {
                return Err(format!("stack workload child failed: {status}"));
            }
        }
        println!(
            "HYPER_STACK_WORKLOAD stage=process operations=4 elapsed_ns={} success=1",
            started.elapsed().as_nanos()
        );
    }
    println!("HYPER_STACK_WORKLOAD_OK");
    Ok(())
}

fn memory_iteration(root: HandleRef<'_, VmarObject>, iteration: u8) -> Result<(), String> {
    const BASE: u64 = 0xd400_0000;
    const SIZE: u64 = 8 * PAGE_SIZE;
    let source = WritableVmo::create(SIZE).map_err(show)?;
    source
        .write_all_at(0, &vec![iteration; SIZE as usize])
        .map_err(show)?;
    let snapshot = SnapshotVmo::from_vmo(source.as_handle_ref()).map_err(show)?;
    source.write_all_at(PAGE_SIZE, &[0xff]).map_err(show)?;
    let options = PrivateMappingOptions {
        source_offset: 0,
        source_length: SIZE - 36,
        data_offset: 17,
        size: SIZE,
        writable: true,
        mode: PrivateMappingMode::CopyOnWrite,
    };
    // SAFETY: these disjoint fixed ranges belong exclusively to this test.
    // Both mappings are explicitly closed before the next iteration reuses VA.
    let mut private = unsafe { snapshot.map_private_at(root, BASE, options) }.map_err(show)?;
    // SAFETY: this second range is disjoint and has the same unique ownership.
    let eager = unsafe {
        snapshot.map_private_at(
            root,
            BASE + 0x10000,
            PrivateMappingOptions {
                mode: PrivateMappingMode::Eager,
                ..options
            },
        )
    }
    .map_err(show)?;
    assert!(private.as_slice()[..17].iter().all(|byte| *byte == 0));
    assert!(
        private.as_slice()[17..SIZE as usize - 19]
            .iter()
            .all(|byte| *byte == iteration)
    );
    assert!(
        private.as_slice()[SIZE as usize - 19..]
            .iter()
            .all(|byte| *byte == 0)
    );
    assert_eq!(private.as_slice(), eager.as_slice());
    private.as_mut_slice().map_err(show)?[PAGE_SIZE as usize] = 0xfe;
    assert_eq!(private.as_slice()[PAGE_SIZE as usize], 0xfe);
    assert_eq!(eager.as_slice()[PAGE_SIZE as usize], iteration);
    let mut original = [0xff];
    snapshot
        .read_exact_at(PAGE_SIZE, &mut original)
        .map_err(show)?;
    assert_eq!(original, [iteration]);
    eager.try_close().map_err(show)?;
    private.try_close().map_err(show)?;
    Ok(())
}
