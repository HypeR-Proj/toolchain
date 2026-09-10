/* SPDX-FileCopyrightText: 2026 roolrz
 * SPDX-License-Identifier: Apache-2.0
 */

ENTRY(_start)

PHDRS
{
    headers PT_PHDR PHDRS;
    image_headers PT_LOAD FILEHDR PHDRS FLAGS(4);
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
    dynamic PT_DYNAMIC FLAGS(4);
    stack PT_GNU_STACK FLAGS(6);
}

SECTIONS
{
    /* The kernel chooses the static-PIE load bias. Keep the link image at 0. */
    . = SIZEOF_HEADERS;
    .image_identity :
    {
        BYTE(0)
    } :image_headers

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .text : ALIGN(16)
    {
        KEEP(*(.text._start))
        *(.text .text.*)
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .dynsym :
    {
        *(.dynsym)
    } :rodata

    .dynstr :
    {
        *(.dynstr)
    } :rodata

    .gnu.hash :
    {
        *(.gnu.hash)
    } :rodata

    .hash :
    {
        *(.hash)
    } :rodata

    .rela.dyn :
    {
        *(.rela.dyn)
    } :rodata

    .relr.dyn :
    {
        *(.relr.dyn)
    } :rodata

    .dynamic :
    {
        *(.dynamic)
    } :rodata :dynamic

    .rodata :
    {
        *(.rodata .rodata.* .srodata .srodata.*)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : ALIGN(16)
    {
        *(.data .data.* .sdata .sdata.*)
    } :data

    /* PIC Rust core/alloc may introduce a GOT. Keep relocation targets in
     * writable data and never let an orphan extend the PT_DYNAMIC table. */
    .got : ALIGN(8)
    {
        *(.got .got.*)
    } :data

    .bss (NOLOAD) : ALIGN(16)
    {
        *(.bss .bss.* .sbss .sbss.*)
        *(COMMON)
    } :data

    /DISCARD/ :
    {
        *(.comment)
        *(.note .note.*)
        *(.eh_frame .eh_frame_hdr)
    }
}

ASSERT(DEFINED(_start), "HypeR Native image requires the CRT entry point")
