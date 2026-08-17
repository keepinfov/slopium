/* The layout, which is the whole reason `[build] linker-script` exists: without
 * it the link takes the toolchain's default script, and a program that decides
 * where its own code goes cannot.
 *
 * `ENTRY` names the stub in `boot/start.s`, which is also what keeps its section
 * alive under `--gc-sections`. `.text.boot` is placed first and by name, because
 * a boot entry that has to be the lowest address is the ordinary reason to write
 * one of these at all; everything after it is the per-function sections v0.8.3
 * emits, collected by wildcard.
 *
 * The load address is an ordinary one for a static, non-PIE x86-64 executable,
 * so this fixture runs on the host as well as proving the flag reached `cc`. */

ENTRY(_start)

SECTIONS
{
	. = 0x400000 + SIZEOF_HEADERS;

	.text : {
		*(.text.boot)
		*(.text .text.*)
	}

	.rodata : ALIGN(16) {
		*(.rodata .rodata.*)
	}

	.data : ALIGN(16) {
		*(.data .data.*)
	}

	.bss : ALIGN(16) {
		*(.bss .bss.*)
		*(COMMON)
	}

	/DISCARD/ : {
		*(.comment)
		*(.note .note.*)
		*(.eh_frame .eh_frame_hdr)
	}
}
