/* The layout of a kernel, which is the first program in this repository whose
 * load address is decided by what loads it rather than by what runs it.
 *
 * 1 MiB is where a multiboot loader puts a kernel, and it is also why this
 * script is not `bare.ld` with one number changed: everything here has to stay
 * inside the first 4 GiB, because the image is handed to `qemu-system-x86_64`
 * as a 32-bit ELF (see `scripts/kernel-check.sh`) and because the compiler
 * emits the small code model and has no `-mcmodel=kernel` to offer. A
 * higher-half kernel is a later question.
 *
 * `.multiboot` is first and is `KEEP`, because nothing references it — it is
 * read by the loader before anything runs, and `--gc-sections` would otherwise
 * be right to drop it. `.text.boot` follows, as in `bare`, and after it come the
 * per-function sections v0.8.3 emits, collected by wildcard.
 *
 * `__bss_start` and `__bss_end` exist because the stub zeroes `.bss` itself
 * rather than trusting the loader to: the bump allocator in `boot/hooks.c`
 * keeps its cursor there, and a cursor that starts at a garbage value is an
 * allocator that hands out garbage. */

ENTRY(_start)

SECTIONS
{
	. = 0x100000 + SIZEOF_HEADERS;

	.multiboot : {
		KEEP(*(.multiboot))
	}

	.text : ALIGN(16) {
		*(.text.boot)
		*(.text .text.*)
	}

	.rodata : ALIGN(16) {
		*(.rodata .rodata.*)
	}

	.data : ALIGN(16) {
		*(.data .data.*)
	}

	.bss : ALIGN(4096) {
		__bss_start = .;
		*(.bss .bss.*)
		*(COMMON)
		. = ALIGN(4096);
		__bss_end = .;
	}

	/DISCARD/ : {
		*(.comment)
		*(.note .note.*)
		*(.eh_frame .eh_frame_hdr)
	}
}
