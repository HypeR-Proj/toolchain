/* SPDX-FileCopyrightText: 2026 roolrz
 * SPDX-License-Identifier: Apache-2.0
 */

#include <hyper/native.h>
#include <hyper/startup.h>
#include <hyper/syscall.h>
#include <string.h>

int hyper_main(const hyper_startup_t *startup)
{
    uint64_t memory = startup->argument_count;
    memset(&memory, 0, sizeof(memory));
    return memory == 0 ? 0 : 1;
}
