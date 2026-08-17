/* Port-mapped I/O, which is the one thing a raw pointer cannot express.
 *
 * `volatile-read` and `volatile-write` reach memory, and a PC serial port is not
 * memory: it answers `in` and `out` on a separate address space that no pointer
 * names. So the two instructions cross the C boundary as ordinary functions and
 * the driver above them is written in Slopium — which is `D-064`'s arrangement,
 * the dangerous half in a file the project supplies, and it is why this fixture
 * needs no language feature that v0.8 did not already have.
 *
 * Making `in` and `out` operators instead was considered and refused: they are
 * x86 and nothing else, and `lowering.rs` is target-neutral by `D-025`. When
 * they arrive they arrive with atomics, in the bare-metal kit.
 *
 * System V puts the first argument in `rdi` and the second in `rsi`, and both
 * `in` and `out` want the port in `dx`. */

	.text
	.code64

	.globl slop_outb
	.type slop_outb, @function
slop_outb:				/* (port: u16, value: u8) -> unit */
	movw	%di, %dx
	movl	%esi, %eax
	outb	%al, (%dx)
	ret
	.size slop_outb, .-slop_outb

	.globl slop_inb
	.type slop_inb, @function
slop_inb:				/* (port: u16) -> u8 */
	movw	%di, %dx
	xorl	%eax, %eax		/* a narrow value is held zero-extended
					 * in a full word (`D-113`), so the
					 * upper half is cleared here rather
					 * than left to the caller */
	inb	(%dx), %al
	ret
	.size slop_inb, .-slop_inb

	.section .note.GNU-stack, "", @progbits
