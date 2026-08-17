/* The entry point, which a freestanding program supplies itself: the compiler
 * emits no `main(argc, argv)` wrapper when the environment has no C start-up to
 * be called from (`D-081`).
 *
 * The Slopium entry is reached by the name it links under — `sl_fn_` followed by
 * the bytes of `main` in hexadecimal, which is `sl_fn_6d61696e`. That mangling
 * is `lowering.rs`'s `function_symbol`, and it is spelled out here rather than
 * computed so that changing it fails at this link instead of quietly resolving
 * to something else.
 *
 * Linux hands `_start` a usable stack, so there is none to set up. A kernel has
 * to set up its own, and that is v0.8.5's business rather than this fixture's.
 *
 * `.text.boot` is a section name the set could not hold before v0.8.3. */

	.section .text.boot, "ax", @progbits
	.globl _start
	.type _start, @function
_start:
	call	sl_fn_6d61696e
	movq	%rax, %rdi		/* what `main` returned is the exit status */
	movl	$60, %eax		/* __NR_exit */
	syscall
	hlt
	.size _start, .-_start

/* Hand-written assembly says this for itself. Without it the linker assumes the
 * object wants an executable stack and marks the whole program that way — the
 * compiler's own objects carry the same marker. */
	.section .note.GNU-stack, "", @progbits
