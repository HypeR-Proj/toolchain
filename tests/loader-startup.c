/* SPDX-FileCopyrightText: 2026 roolrz
 * SPDX-License-Identifier: Apache-2.0
 */
/* Exercise the shipped parser and stack encoder with only syscalls replaced. */
#include <assert.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <hyper/syscall.h>
#if defined(TEST_RISCV)
#undef __aarch64__
#define __riscv 1
#define __riscv_xlen 64
#else
#undef __riscv
#define __aarch64__ 1
#endif
#ifdef TEST_DYNAMIC_LOADER
#include "../../../lib/dynamic-loader/elf/image.c"
#include "../../../lib/dynamic-loader/elf/launch.c"
#include "../../../lib/dynamic-loader/src/bootstrap.c"
const Elf64_Ehdr __ehdr_start = {.type = 2, .phentsize = sizeof(Elf64_Phdr)};
#else
#include "../../../bootstrap/stage1/userspace-loader/elf/image.c"
#include "../../../bootstrap/stage1/userspace-loader/elf/launch.c"
#include "../../../bootstrap/stage1/userspace-loader/src/start.c"
#endif
#include <hyper/relocate.h>

static _Alignas(16) unsigned char file_bytes[16384];
static _Alignas(16) unsigned char initial_stack[128 * 1024];
static _Alignas(16) unsigned char packet[MESSAGE_BYTES];
static size_t packet_size;
static size_t mappings, closes;
static int map_failure;
static uint32_t expected_mapping_mode = HYPER_NATIVE_PRIVATE_MAPPING_COPY_ON_WRITE;
static int protection_live, deny_protect;
static size_t read_limit;
static jmp_buf handoff;
static uintptr_t entered_stack, entered_entry;
static int entered_dynamic;
static int64_t exit_status, reported_status;

static Elf64_Phdr *headers(void)
{
	return (void *)(file_bytes + sizeof(Elf64_Ehdr));
}

static void reset_image(void)
{
	memset(file_bytes, 0, sizeof(file_bytes));
	Elf64_Ehdr *eh = (void *)file_bytes;
	memcpy(eh->ident, "\177ELF\2\1\1\77", 8);
	eh->type = 3;
#ifdef TEST_RISCV
	eh->machine = 243;
	eh->flags = 5;
#else
	eh->machine = 183;
#endif
	eh->version = 1;
	eh->entry = 4096;
	eh->ehsize = sizeof(*eh);
	eh->phoff = sizeof(*eh);
	eh->phentsize = sizeof(Elf64_Phdr);
	eh->phnum = 3;
	headers()[0] =
		(Elf64_Phdr){.type = 1, .flags = 4, .filesz = 4096, .memsz = 4096, .align = 4096};
	headers()[1] = (Elf64_Phdr){.type = 1,
				    .flags = 5,
				    .offset = 4096,
				    .vaddr = 4096,
				    .filesz = 4096,
				    .memsz = 4096,
				    .align = 4096};
	headers()[2] = (Elf64_Phdr){.type = 1,
				    .flags = 6,
				    .offset = 8192,
				    .vaddr = 8192,
				    .filesz = 17,
				    .memsz = 8192,
				    .align = 4096};
	mappings = closes = 0;
	map_failure = 0;
	protection_live = deny_protect = 0;
	read_limit = SIZE_MAX;
}

hyper_call_result_t hyper_file_read_at(hyper_native_handle_t vmo, uint64_t offset, void *out,
				       size_t size)
{
	assert(protection_live);
	if (size > read_limit)
		size = read_limit;
	assert(vmo == 41 && offset <= sizeof(file_bytes) && size <= sizeof(file_bytes) - offset);
	memcpy(out, file_bytes + offset, size);
	return (hyper_call_result_t){.value0 = size};
}

hyper_call_result_t hyper_vmar_allocate(hyper_native_handle_t root, uintptr_t address, size_t size,
					uint64_t options)
{
	assert(root == 42 && address == 0x400000 && size == sizeof(file_bytes));
	assert(options == HYPER_NATIVE_VMAR_ALLOCATE_EXACT);
	return (hyper_call_result_t){.value0 = 50};
}

hyper_native_status_t hyper_vmar_map_file(hyper_native_handle_t vmar, hyper_native_handle_t file,
					  hyper_native_handle_t protection,
					  const hyper_native_private_mapping_t *request,
					  size_t size)
{
	assert(protection_live);
	assert(vmar == 42 && file == 41 && protection == 45 && size == sizeof(*request));
	assert(request->mode == expected_mapping_mode);
	assert(!(request->permissions & HYPER_NATIVE_VMAR_PERMISSION_WRITE) ||
	       !(request->permissions & HYPER_NATIVE_VMAR_PERMISSION_EXECUTE));
	++mappings;
	return map_failure ? HYPER_NATIVE_STATUS_NO_MEMORY : 0;
}

hyper_native_status_t hyper_handle_close(hyper_native_handle_t value)
{
	assert(value != 0);
	if (value == 45) {
		assert(protection_live);
		protection_live = 0;
	}
	++closes;
	return 0;
}

hyper_call_result_t hyper_directory_open_file(hyper_native_handle_t dir, const void *path,
					      size_t size, uint64_t rights)
{
	(void)dir;
	(void)path;
	(void)size;
	(void)rights;
	return (hyper_call_result_t){.status = HYPER_NATIVE_STATUS_NOT_FOUND};
}

hyper_call_result_t hyper_file_protect(hyper_native_handle_t file)
{
	assert(file == 41 && !protection_live);
	if (deny_protect)
		return (hyper_call_result_t){.status = HYPER_NATIVE_STATUS_ACCESS_DENIED};
	protection_live = 1;
	return (hyper_call_result_t){.value0 = 45, .value1 = sizeof(file_bytes)};
}

hyper_call_result_t hyper_byte_channel_read(hyper_native_handle_t channel, void *out, size_t size)
{
	assert(channel == 43 && packet_size <= size);
	memcpy(out, packet, packet_size);
	return (hyper_call_result_t){.value0 = packet_size};
}

hyper_native_status_t hyper_byte_channel_write(hyper_native_handle_t channel, const void *bytes,
					       size_t size)
{
	assert(channel == 43 && size == sizeof(hyper_launch_result_t));
	reported_status = ((const hyper_launch_result_t *)bytes)->status;
	return 0;
}

hyper_call_result_t hyper_native_invoke6(uint64_t receiver, uint64_t operation, uint64_t status,
					 uint64_t b, uint64_t c, uint64_t d, uint64_t e, uint64_t f)
{
	assert(receiver == 46 && operation == HYPER_NATIVE_OP_PROCESS_EXIT);
	assert(!b && !c && !d && !e && !f);
	exit_status = (int64_t)status;
	longjmp(handoff, 1);
}

hyper_native_status_t hyper_native_bind_invocation_handles(uint64_t execution, uint64_t factory,
							   uint64_t system)
{
	assert(execution == 46 && factory == 47 && system == 48);
	return HYPER_NATIVE_STATUS_OK;
}

#ifdef TEST_DYNAMIC_LOADER
_Noreturn void __hyper_rtld_link(const ElfImage *image, const ElfLaunch *launch,
				 hyper_native_handle_t channel)
{
	entered_dynamic = 1;
	entered_stack = elf_launch_stack(launch, image, channel, launch->boot->loader_base, 0);
	entered_entry = image->entry;
	longjmp(handoff, 1);
}

#define __hyper_userspace_enter __hyper_rtld_enter
#define __hyper_userspace_start __hyper_rtld_start
#endif
_Noreturn void __hyper_userspace_enter(uintptr_t stack, uintptr_t entry)
{
	entered_stack = stack;
	entered_entry = entry;
	longjmp(handoff, 1);
}

static void prepare_packet(void)
{
	memset(packet, 0, sizeof(packet));
	hyper_native_loader_startup_t *boot = (void *)packet;
	*boot = (hyper_native_loader_startup_t){.size = sizeof(*boot),
						.flags = HYPER_NATIVE_LOADER_STARTUP_REPLY,
						.executable = 41,
						.root_vmar = 42,
						.stack_vmar = 44,
						.execution_context = 46,
						.object_factory = 47,
						.system_info = 48,
						.stack_base = (uintptr_t)initial_stack,
						.stack_size = sizeof(initial_stack),
						.loader_base = 0x100000,
						.loader_size = 0x5000};
	packet_size = sizeof(*boot);
	entered_stack = entered_entry = 0;
	entered_dynamic = 0;
	exit_status = reported_status = 0;
}

static void run_loader(void)
{
	if (!setjmp(handoff))
		__hyper_userspace_start(43, (uintptr_t)initial_stack + sizeof(initial_stack));
}

static hyper_native_status_t load_test_image(ElfImage *image)
{
	return elf_image_load(image, 42, 41, 0x400000, 0x10000000, 0);
}

static void parser_cases(void)
{
	ElfImage image;
	reset_image();
	deny_protect = 1;
	assert(load_test_image(&image) == HYPER_NATIVE_STATUS_ACCESS_DENIED);
	assert(mappings == 0 && closes == 0);
	reset_image();
	read_limit = 7; /* Protected reads may complete short, even for ELF headers. */
	assert(load_test_image(&image) == 0);
	assert(mappings == 3 && closes == 1 && !protection_live);
	reset_image();
	assert(load_test_image(&image) == 0);
	assert(mappings == 3 && image.entry == 0x401000 && image.program_headers == 0x400040);
	reset_image();
	headers()[2].flags = 7;
	assert(load_test_image(&image) == BAD_IMAGE);
	assert(mappings == 0);
	reset_image();
	headers()[2].vaddr = 4096;
	assert(load_test_image(&image) == BAD_IMAGE);
	reset_image();
	headers()[2].filesz = sizeof(file_bytes);
	assert(load_test_image(&image) == BAD_IMAGE);
	reset_image();
	((Elf64_Ehdr *)file_bytes)->entry = 8192;
	assert(load_test_image(&image) == BAD_IMAGE);
	reset_image();
	headers()[2].vaddr = UINT64_MAX - 5;
	assert(load_test_image(&image) == BAD_IMAGE);
	reset_image();
	map_failure = 1;
	assert(load_test_image(&image) == HYPER_NATIVE_STATUS_NO_MEMORY);
	assert(closes == 1);
}

static void stack_cases(void)
{
	reset_image();
	prepare_packet();
	run_loader();
	assert(!exit_status && !entered_dynamic && entered_entry == 0x401000 &&
	       !(entered_stack % 16));
	const uintptr_t *stack = (const void *)entered_stack;
	assert(stack[0] == 1 && !strcmp((const char *)stack[1], "/init") && !stack[2] && !stack[3]);
	assert(hyper_elf_auxiliary(stack, HYPER_AUXV_STARTUP_CHANNEL) == 43);
	assert(hyper_elf_auxiliary(stack, 7) == 0);
	assert(hyper_elf_auxiliary(stack, HYPER_AUXV_LOADER_SIZE) == 0x5000);
	assert(hyper_elf_auxiliary(stack, HYPER_AUXV_STARTUP_HANDLE_COUNT) == 5);
	const hyper_native_startup_handle_t *handles =
		(const void *)hyper_elf_auxiliary(stack, HYPER_AUXV_STARTUP_HANDLES);
	assert(handles[0].handle == 42 && handles[1].handle == 44);
	assert(handles[2].handle == 46 && handles[3].handle == 47 && handles[4].handle == 48);
	assert(handles[2].purpose == HYPER_NATIVE_STARTUP_HANDLE_PURPOSE_EXECUTION_CONTEXT);
	assert(handles[3].purpose == HYPER_NATIVE_STARTUP_HANDLE_PURPOSE_OBJECT_FACTORY);
	assert(handles[4].purpose == HYPER_NATIVE_STARTUP_HANDLE_PURPOSE_SYSTEM_INFO);
	reset_image();
	prepare_packet();
	hyper_native_loader_startup_t *boot = (void *)packet;
	boot->data_size = 24;
	packet_size += 24;
	hyper_launch_data_t *data = (void *)(boot + 1);
	*data = (hyper_launch_data_t){1, 1};
	uint32_t *offsets = (void *)(data + 1);
	offsets[0] = 16;
	offsets[1] = 20;
	memcpy((char *)data + 16, "app\0X=1", 8);
	run_loader();
	stack = (const void *)entered_stack;
	assert(entered_stack && !strcmp((const char *)stack[1], "app") &&
	       !strcmp((const char *)stack[3], "X=1"));
	offsets[1] = 24;
	entered_stack = 0;
	run_loader();
	assert(!entered_stack && exit_status == BAD_IMAGE && reported_status == BAD_IMAGE);
	reset_image();
	prepare_packet();
	boot->handle_count = 1;
	packet_size += 16;
	((hyper_native_startup_handle_t *)(boot + 1))[0] = (hyper_native_startup_handle_t){
		.purpose = HYPER_NATIVE_STARTUP_HANDLE_PURPOSE_ROOT_VMAR, .handle = 60};
	run_loader();
	assert(!entered_stack && exit_status == BAD_IMAGE);
	reset_image();
	prepare_packet();
	map_failure = 1;
	run_loader();
	assert(exit_status == HYPER_NATIVE_STATUS_NO_MEMORY && reported_status == exit_status);
}

static void maximum_packet(void)
{
	reset_image();
	prepare_packet();
	hyper_native_loader_startup_t *boot = (void *)packet;
	boot->handle_count = HYPER_NATIVE_STARTUP_MAX_HANDLES - 5;
	boot->data_size = HYPER_NATIVE_PROCESS_STARTUP_DATA_MAX_BYTES;
	hyper_native_startup_handle_t *handles = (void *)(boot + 1);
	for (size_t i = 0; i < boot->handle_count; ++i)
		handles[i] = (hyper_native_startup_handle_t){.purpose = 0x90000000 + i,
							     .handle = 100 + i};
	hyper_launch_data_t *data = (void *)(handles + boot->handle_count);
	*data = (hyper_launch_data_t){HYPER_LAUNCH_MAX_ARGUMENTS, HYPER_LAUNCH_MAX_ENVIRONMENT};
	uint32_t *offsets = (void *)(data + 1);
	size_t count = data->argument_count + data->environment_count;
	size_t strings = sizeof(*data) + count * sizeof(*offsets);
	for (size_t i = 0; i < count; ++i)
		offsets[i] = strings;
	memcpy((char *)data + strings, "A=B", 4);
	packet_size = sizeof(*boot) + boot->handle_count * 16 + boot->data_size;
	run_loader();
	assert(!exit_status && entered_entry == 0x401000);
	assert(entered_stack >= (uintptr_t)initial_stack + sizeof(initial_stack) - 65536);
	assert(hyper_elf_auxiliary((void *)entered_stack, HYPER_AUXV_STARTUP_HANDLE_COUNT) ==
	       HYPER_NATIVE_STARTUP_MAX_HANDLES);
}

static void interpreter_cases(void)
{
	reset_image();
	prepare_packet();
	Elf64_Ehdr *eh = (void *)file_bytes;
	eh->phnum = 4;
#ifdef TEST_RISCV
	const char *path = "/lib64/ld-hyper-riscv64.so";
#else
	const char *path = "/lib64/ld-hyper-aarch64.so";
#endif
	headers()[3] = (Elf64_Phdr){.type = 3, .offset = 512, .filesz = strlen(path) + 1};
	memcpy(file_bytes + 512, path, strlen(path) + 1);
	run_loader();
#ifdef TEST_DYNAMIC_LOADER
	assert(!exit_status && entered_dynamic && mappings == 3);
	const uintptr_t *stack = (const void *)entered_stack;
	assert(hyper_elf_auxiliary(stack, 7) == 0x100000);
	assert(hyper_elf_auxiliary(stack, HYPER_AUXV_LOADER_SIZE) == 0);
	assert(hyper_elf_auxiliary(stack, HYPER_AUXV_STARTUP_CHANNEL) == 43);
#else
	assert(exit_status == HYPER_NATIVE_STATUS_NOT_SUPPORTED && mappings == 0);
#endif
	/* Selecting a loader capability does not authorize following another path. */
	prepare_packet();
	mappings = 0;
	file_bytes[519] = 'X';
	run_loader();
	assert(exit_status == HYPER_NATIVE_STATUS_NOT_SUPPORTED && mappings == 0);
}

static void relocation_cases(void)
{
	reset_image();
	Elf64_Ehdr *eh = (void *)file_bytes;
	eh->phnum = 4;
	headers()[3] = (Elf64_Phdr){.type = 2, .vaddr = 512, .filesz = 4 * sizeof(Elf64_Dyn)};
	Elf64_Dyn *dynamic = (void *)(file_bytes + 512);
	dynamic[0] = (Elf64_Dyn){7, 1024};
	dynamic[1] = (Elf64_Dyn){8, sizeof(Elf64_Rela)};
	dynamic[2] = (Elf64_Dyn){9, sizeof(Elf64_Rela)};
	Elf64_Rela *rela = (void *)(file_bytes + 1024);
#ifdef TEST_RISCV
	*rela = (Elf64_Rela){8192, 3, 123};
#else
	*rela = (Elf64_Rela){8192, 1027, 123};
#endif
	assert(hyper_elf_self_relocate(eh));
	assert(*(uintptr_t *)(file_bytes + 8192) == (uintptr_t)file_bytes + 123);
	rela->offset = 4096; /* executable destination must never be rewritten */
	assert(!hyper_elf_self_relocate(eh));
	rela->offset = 8193;
	assert(!hyper_elf_self_relocate(eh));
	rela->offset = 8192;
	rela->info |= UINT64_C(1) << 32;
	assert(!hyper_elf_self_relocate(eh));
	/* Packed relocations: address entry followed by a sparse bitmap. */
	reset_image();
	eh->phnum = 4;
	headers()[3] = (Elf64_Phdr){.type = 2, .vaddr = 512, .filesz = 4 * sizeof(Elf64_Dyn)};
	dynamic[0] = (Elf64_Dyn){36, 1024};
	dynamic[1] = (Elf64_Dyn){35, 16};
	dynamic[2] = (Elf64_Dyn){37, 8};
	uint64_t *relr = (void *)(file_bytes + 1024);
	relr[0] = 8192;
	relr[1] = 1 | (1 << 1) | (1 << 3);
	uintptr_t *targets = (void *)(file_bytes + 8192);
	targets[0] = 12;
	targets[1] = 24;
	targets[2] = 36;
	targets[3] = 48;
	assert(hyper_elf_self_relocate(eh));
	assert(targets[0] == (uintptr_t)file_bytes + 12);
	assert(targets[1] == (uintptr_t)file_bytes + 24);
	assert(targets[2] == 36);
	assert(targets[3] == (uintptr_t)file_bytes + 48);
	relr[0] = 3; /* A bitmap cannot precede an address. */
	assert(!hyper_elf_self_relocate(eh));
	relr[0] = 8192;
	relr[1] = 8192; /* Repeated/backward targets would add the bias twice. */
	assert(!hyper_elf_self_relocate(eh));
	relr[1] = 1;
	dynamic[2] = dynamic[0]; /* Duplicate relocation metadata is ambiguous. */
	assert(!hyper_elf_self_relocate(eh));
}

int main(void)
{
	parser_cases();
	stack_cases();
	interpreter_cases();
	maximum_packet();
	relocation_cases();
	puts("userspace ELF admission, startup handoff and self relocation passed");
	return 0;
}
