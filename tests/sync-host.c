/* SPDX-FileCopyrightText: 2026 roolrz
 * SPDX-License-Identifier: Apache-2.0
 */
#include <hyper/heap.h>
#include <hyper/syscall.h>
#include <sched.h>
#include <time.h>

/* This fixture uses the host allocator, so thread teardown has no Native
 * arena cache to release. The actual heap is covered by libhyper unit tests. */
size_t hyper_heap_trim(void)
{
	return 0;
}

hyper_native_status_t hyper_thread_yield(void)
{
	sched_yield();
	return 0;
}

hyper_call_result_t hyper_clock_get_monotonic(void)
{
	struct timespec now;
	if (clock_gettime(CLOCK_MONOTONIC, &now) != 0)
		__builtin_trap();
	return (hyper_call_result_t){0, (uint64_t)now.tv_sec * 1000000000 + now.tv_nsec, 0};
}

_Noreturn void hyper_process_exit(int64_t code)
{
	(void)code;
	__builtin_trap();
}

uint64_t __hyper_std_clock(void)
{
	return hyper_clock_get_monotonic().value0;
}
