/* SPDX-FileCopyrightText: 2026 roolrz
 * SPDX-License-Identifier: Apache-2.0
 */

#include <hyper/syscall.h>

__attribute__((visibility("default"))) hyper_native_status_t hyper_shared_smoke(void)
{
    return hyper_thread_yield();
}
