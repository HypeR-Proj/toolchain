/* SPDX-FileCopyrightText: 2026 roolrz
 * SPDX-License-Identifier: Apache-2.0
 */

#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <hyper/native.h>

enum {
    ELF_HEADER_SIZE = 64,
    ELF_PROGRAM_HEADER_SIZE = 56,
    ELF_CLASS_64 = 2,
    ELF_DATA_LITTLE_ENDIAN = 1,
    ELF_VERSION_CURRENT = 1,
    ELF_TYPE_DYNAMIC = 3,
    ELF_MACHINE_AARCH64 = 183,
    ELF_PROGRAM_LOAD = 1,
    ELF_PROGRAM_INTERPRETER = 3,
    ELF_PROGRAM_TLS = 7,
    ELF_PROGRAM_GNU_STACK = 0x6474e551,
    ELF_FLAG_EXECUTE = 1,
    ELF_FLAG_WRITE = 2,
    ELF_FLAG_READ = 4,
    ELF_PAGE_SIZE = 4096,
};

struct load_range {
    uint64_t start;
    uint64_t end;
};

static uint16_t read_u16(const unsigned char *bytes)
{
    return (uint16_t)bytes[0] | (uint16_t)((uint16_t)bytes[1] << 8);
}

static uint32_t read_u32(const unsigned char *bytes)
{
    return (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
           (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
}

static uint64_t read_u64(const unsigned char *bytes)
{
    return (uint64_t)read_u32(bytes) | (uint64_t)read_u32(bytes + 4) << 32;
}

static int fail(const char *path, const char *message)
{
    fprintf(stderr, "hyper-brand-elf: %s: %s\n", path, message);
    return EXIT_FAILURE;
}

static int validate_program_headers(
    FILE *file,
    const char *path,
    uint64_t file_size,
    uint64_t offset,
    uint16_t entry_size,
    uint16_t count,
    uint64_t entry_point)
{
    unsigned char header[ELF_PROGRAM_HEADER_SIZE];
    struct load_range ranges[128];
    uint16_t range_count = 0;
    int executable_load = 0;
    int executable_entry = 0;
    int stack_policy_headers = 0;

    if (entry_size != ELF_PROGRAM_HEADER_SIZE || count == 0 || count > 128) {
        return fail(path, "invalid program-header table geometry");
    }
    for (uint16_t index = 0; index < count; ++index) {
        const uint64_t relative_offset = (uint64_t)index * entry_size;
        if (relative_offset > UINT64_MAX - offset) {
            return fail(path, "program-header table offset overflows");
        }
        const uint64_t entry_offset = offset + relative_offset;

        if (entry_offset > (uint64_t)LONG_MAX ||
            entry_offset > file_size || file_size - entry_offset < sizeof(header) ||
            fseek(file, (long)entry_offset, SEEK_SET) != 0 ||
            fread(header, 1, sizeof(header), file) != sizeof(header)) {
            return fail(path, "truncated program-header table");
        }
        const uint32_t type = read_u32(header);
        const uint32_t flags = read_u32(header + 4);
        if (type == ELF_PROGRAM_INTERPRETER) {
            return fail(path, "dynamic interpreter is not permitted");
        }
        if (type == ELF_PROGRAM_TLS && read_u64(header + 40) != 0) {
            return fail(path, "thread-local storage is not yet supported");
        }
        if (type == ELF_PROGRAM_GNU_STACK) {
            if (++stack_policy_headers != 1) {
                return fail(path, "multiple stack-policy headers are not permitted");
            }
            if ((flags & ELF_FLAG_EXECUTE) != 0) {
                return fail(path, "executable stack is not permitted");
            }
        }
        if (type != ELF_PROGRAM_LOAD) {
            continue;
        }
        if ((flags & ~(ELF_FLAG_READ | ELF_FLAG_WRITE | ELF_FLAG_EXECUTE)) != 0 ||
            (flags & ELF_FLAG_READ) == 0) {
            return fail(path, "invalid load-segment permissions");
        }
        if ((flags & (ELF_FLAG_WRITE | ELF_FLAG_EXECUTE)) ==
            (ELF_FLAG_WRITE | ELF_FLAG_EXECUTE)) {
            return fail(path, "writable executable load segment is not permitted");
        }
        const uint64_t file_offset = read_u64(header + 8);
        const uint64_t virtual_address = read_u64(header + 16);
        const uint64_t data_size = read_u64(header + 32);
        const uint64_t memory_size = read_u64(header + 40);
        const uint64_t alignment = read_u64(header + 48);
        if (data_size > memory_size || file_offset > file_size ||
            data_size > file_size - file_offset) {
            return fail(path, "invalid load-segment file range");
        }
        if ((alignment > 1 &&
             ((alignment & (alignment - 1)) != 0 ||
              virtual_address % alignment != file_offset % alignment)) ||
            virtual_address % ELF_PAGE_SIZE != file_offset % ELF_PAGE_SIZE) {
            return fail(path, "invalid load-segment alignment");
        }
        if (memory_size > UINT64_MAX - virtual_address) {
            return fail(path, "load-segment address overflows");
        }
        const uint64_t memory_end = virtual_address + memory_size;
        if (memory_size != 0) {
            const uint64_t mapping_start = virtual_address & ~(uint64_t)(ELF_PAGE_SIZE - 1);
            if (memory_end > UINT64_MAX - (ELF_PAGE_SIZE - 1)) {
                return fail(path, "load-segment mapping overflows");
            }
            ranges[range_count++] = (struct load_range){
                .start = mapping_start,
                .end = (memory_end + ELF_PAGE_SIZE - 1) & ~(uint64_t)(ELF_PAGE_SIZE - 1),
            };
        }
        executable_load |= (flags & ELF_FLAG_EXECUTE) != 0;
        executable_entry |= (flags & ELF_FLAG_EXECUTE) != 0 &&
                            virtual_address <= entry_point && entry_point < memory_end;
    }
    if (!executable_load) {
        return fail(path, "image must contain an executable load segment");
    }
    if (!executable_entry) {
        return fail(path, "entry point is not contained in an executable load segment");
    }
    if (stack_policy_headers != 1) {
        return fail(path, "explicit non-executable stack policy is required");
    }
    for (uint16_t left = 0; left < range_count; ++left) {
        for (uint16_t right = left + 1; right < range_count; ++right) {
            if (ranges[left].start < ranges[right].end &&
                ranges[right].start < ranges[left].end) {
                return fail(path, "load-segment mappings overlap");
            }
        }
    }
    return EXIT_SUCCESS;
}

static int brand(const char *path, int check_only)
{
    unsigned char header[ELF_HEADER_SIZE];
    FILE *file = fopen(path, check_only ? "rb" : "r+b");
    uint64_t file_size;

    if (file == NULL) {
        fprintf(stderr, "hyper-brand-elf: %s: %s\n", path, strerror(errno));
        return EXIT_FAILURE;
    }
    if (fseek(file, 0, SEEK_END) != 0) {
        fclose(file);
        return fail(path, "cannot determine image size");
    }
    const long end = ftell(file);
    if (end < 0 || fseek(file, 0, SEEK_SET) != 0) {
        fclose(file);
        return fail(path, "cannot determine image size");
    }
    file_size = (uint64_t)end;
    if (fread(header, 1, sizeof(header), file) != sizeof(header)) {
        fclose(file);
        return fail(path, "truncated ELF header");
    }
    if (memcmp(header, "\177ELF", 4) != 0 || header[4] != ELF_CLASS_64 ||
        header[5] != ELF_DATA_LITTLE_ENDIAN || header[6] != ELF_VERSION_CURRENT) {
        fclose(file);
        return fail(path, "not a little-endian ELF64 image");
    }
    if (read_u16(header + 16) != ELF_TYPE_DYNAMIC ||
        read_u16(header + 18) != ELF_MACHINE_AARCH64 ||
        read_u32(header + 20) != ELF_VERSION_CURRENT || read_u32(header + 48) != 0 ||
        read_u16(header + 52) != ELF_HEADER_SIZE) {
        fclose(file);
        return fail(path, "not an AArch64 static PIE image");
    }
    if ((header[7] != 0 && header[7] != HYPER_NATIVE_ELF_OSABI) ||
        header[8] != HYPER_NATIVE_ELF_ABI_VERSION) {
        fclose(file);
        return fail(path, "unexpected input ELF ABI identity");
    }
    if (validate_program_headers(
            file,
            path,
            file_size,
            read_u64(header + 32),
            read_u16(header + 54),
            read_u16(header + 56),
            read_u64(header + 24)) != EXIT_SUCCESS) {
        fclose(file);
        return EXIT_FAILURE;
    }
    if (check_only) {
        const int branded = header[7] == HYPER_NATIVE_ELF_OSABI;
        fclose(file);
        return branded ? EXIT_SUCCESS : fail(path, "image is not branded for HypeR");
    }
    header[7] = (unsigned char)HYPER_NATIVE_ELF_OSABI;
    header[8] = (unsigned char)HYPER_NATIVE_ELF_ABI_VERSION;
    int publication_failed = 0;
    if (fseek(file, 0, SEEK_SET) != 0 ||
        fwrite(header, 1, sizeof(header), file) != sizeof(header)) {
        publication_failed = 1;
    }
    if (fflush(file) != 0) {
        publication_failed = 1;
    }
    if (fclose(file) != 0) {
        publication_failed = 1;
    }
    if (publication_failed) {
        return fail(path, "failed to publish branded ELF header");
    }
    return EXIT_SUCCESS;
}

int main(int argc, char **argv)
{
    if (argc == 2) {
        return brand(argv[1], 0);
    }
    if (argc == 3 && strcmp(argv[1], "--check") == 0) {
        return brand(argv[2], 1);
    }
    {
        fputs("usage: hyper-brand-elf [--check] IMAGE\n", stderr);
        return EXIT_FAILURE;
    }
}
