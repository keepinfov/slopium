/* The boot stub: everything that has to happen before a Slopium function can be
 * called on a bare machine.
 *
 * A multiboot loader hands control over in 32-bit protected mode with paging
 * off, and the compiler emits 64-bit code, so this file is the transition and
 * nothing else. It is the one part of a kernel that cannot be written in
 * Slopium — not because the language is missing a word, but because there is no
 * long mode to run it in yet.
 *
 * The Slopium entry is reached by the name it links under, `sl_fn_6d61696e`,
 * spelled out here rather than computed for the reason `bare/boot/start.s`
 * gives: a changed mangling must fail at this link instead of quietly resolving
 * to something else.
 *
 * Interrupts are never enabled and no IDT is ever loaded. That is what makes the
 * red zone sound — the compiler emits ordinary System V code and may use the 128
 * bytes below `rsp`, which only an asynchronous push would corrupt. A kernel
 * that grows an interrupt handler has to revisit this, and the handler form is
 * v1.4's business. */

/* -------------------------------------------------------------------------
 * The multiboot header, which the loader reads out of the file before any of
 * this runs. Nothing references it, so the linker script `KEEP`s it.
 * ------------------------------------------------------------------------- */

	.set MULTIBOOT_MAGIC, 0x1BADB002
	.set MULTIBOOT_FLAGS, 0x00000000

	.section .multiboot, "a", @progbits
	.align 4
	.long MULTIBOOT_MAGIC
	.long MULTIBOOT_FLAGS
	.long -(MULTIBOOT_MAGIC + MULTIBOOT_FLAGS)

/* -------------------------------------------------------------------------
 * 32-bit entry.
 * ------------------------------------------------------------------------- */

	.section .text.boot, "ax", @progbits
	.code32
	.globl _start
	.type _start, @function
_start:
	cli
	cld

	/* Zero `.bss` first, before anything reads a static. The page tables
	 * below live there and are filled entry by entry afterwards, and the
	 * allocator's cursor in `boot/hooks.c` lives there too. */
	movl	$__bss_start, %edi
	movl	$__bss_end, %ecx
	subl	%edi, %ecx
	xorl	%eax, %eax
	rep stosb

	/* Identity-map the first 8 MiB with 2 MiB pages. `0xB8000` is inside
	 * it, which is the whole requirement: this kernel touches the text
	 * framebuffer, its own image, and nothing else. */
	movl	$pdpt, %eax
	orl	$0x03, %eax		/* present | writable */
	movl	%eax, pml4

	movl	$pd, %eax
	orl	$0x03, %eax
	movl	%eax, pdpt

	movl	$pd, %edi
	movl	$0x00000083, %eax	/* present | writable | page size */
	movl	$4, %ecx
1:
	movl	%eax, (%edi)
	addl	$0x200000, %eax
	addl	$8, %edi
	loop	1b

	/* PAE, then the tables, then long mode enabled, then paging on. The
	 * order is the architecture's and not a preference. */
	movl	%cr4, %eax
	orl	$(1 << 5), %eax		/* CR4.PAE */
	movl	%eax, %cr4

	movl	$pml4, %eax
	movl	%eax, %cr3

	movl	$0xC0000080, %ecx	/* IA32_EFER */
	rdmsr
	orl	$(1 << 8), %eax		/* EFER.LME */
	wrmsr

	movl	%cr0, %eax
	orl	$(1 << 31), %eax	/* CR0.PG */
	movl	%eax, %cr0

	lgdt	gdt_descriptor
	ljmp	$0x08, $long_mode_start

	.size _start, .-_start

/* -------------------------------------------------------------------------
 * 64-bit entry.
 * ------------------------------------------------------------------------- */

	.code64
	.type long_mode_start, @function
long_mode_start:
	movw	$0x10, %ax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %fs
	movw	%ax, %gs
	movw	%ax, %ss

	/* System V wants `rsp` 16-byte aligned at the call boundary, so that it
	 * is 8 modulo 16 once the return address is pushed. */
	movq	$stack_top, %rsp
	andq	$-16, %rsp

	call	sl_fn_6d61696e

	/* What `main` returned leaves through QEMU's `isa-debug-exit` device,
	 * which turns a write into an exit status of `(value << 1) | 1`. That is
	 * how a machine with no operating system under it reports an answer, and
	 * it is why the status can never be zero. */
	outl	%eax, $0xF4

2:
	hlt
	jmp	2b

	.size long_mode_start, .-long_mode_start

/* -------------------------------------------------------------------------
 * The descriptor table. Long mode ignores the base and the limit; what matters
 * is the `L` bit in the code descriptor.
 * ------------------------------------------------------------------------- */

	.section .rodata
	.balign 16
gdt:
	.quad	0x0000000000000000	/* null */
	.quad	0x00AF9A000000FFFF	/* code: present, executable, 64-bit */
	.quad	0x00CF92000000FFFF	/* data: present, writable */
gdt_end:

gdt_descriptor:
	.word	gdt_end - gdt - 1
	.long	gdt

/* -------------------------------------------------------------------------
 * Page tables and the stack. Both are `.bss`, which the entry above zeroes.
 * ------------------------------------------------------------------------- */

	.section .bss, "aw", @nobits
	.balign 4096
pml4:
	.skip 4096
pdpt:
	.skip 4096
pd:
	.skip 4096

	.balign 16
stack_bottom:
	.skip 16384
stack_top:

/* Hand-written assembly says this for itself; without it the linker marks the
 * whole program as wanting an executable stack. */
	.section .note.GNU-stack, "", @progbits
