// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

//! Invoke the kernel directly so SDK argument checks cannot hide wire errors.

pub fn run() -> Result<(), String> {
    for size in [
        0,
        1,
        hyper_abi::HYPER_NATIVE_PAGE_SIZE - 1,
        hyper_abi::HYPER_NATIVE_PAGE_SIZE * 3,
        hyper_abi::HYPER_NATIVE_VMO_MAX_CONTIGUOUS_SIZE_BYTES * 2,
    ] {
        // SAFETY: this call has only scalar inputs; no memory can be borrowed.
        let result = unsafe { hyper_sys::vmo_create_contiguous(size) };
        if result.status != hyper_abi::HYPER_NATIVE_STATUS_INVALID_ARGUMENT
            || result.value0 != 0
            || result.value1 != 0
        {
            // An unexpected successful result still transfers an owned handle.
            if result.status == 0 && result.value0 != 0 {
                // SAFETY: the successful creation returned this unique owner.
                let _ = unsafe { hyper_sys::handle_close(result.value0) };
            }
            return Err(format!("contiguous size {size}: status {}", result.status));
        }
    }
    let valid = hyper_os::memory::WritableVmo::create_contiguous(hyper_abi::HYPER_NATIVE_PAGE_SIZE)
        .map_err(|error| format!("valid contiguous allocation: {error:?}"))?;
    valid
        .write_all_at(0, b"contiguous")
        .map_err(|error| format!("write: {error:?}"))?;
    let mut bytes = [0; 10];
    valid
        .read_exact_at(0, &mut bytes)
        .map_err(|error| format!("read: {error:?}"))?;
    if &bytes != b"contiguous" {
        return Err("contiguous contents changed".into());
    }
    drop(valid);
    println!("HYPER_MEMORY_ABI_CONTRACT_OK");
    Ok(())
}
