/* The four symbols `slop_rt_core.c` calls and does not define (`D-066`,
 * `D-080`). `bare/boot/hooks.c` is the same file with different exits: there,
 * a failure leaves through a Linux `exit` syscall, and here there is no one to
 * make a syscall to. A kernel says why it died on the wire it has, and then
 * stops the machine.
 *
 * The allocator is the same bump allocator over a static arena, which is enough
 * for a program that never frees and exists to prove that `core` — and with it
 * every string literal in the kernel — runs with no C library under it. Its
 * cursor lives in `.bss`, which `boot/start.s` zeroes before anything runs. */

#include <stddef.h>
#include <stdint.h>

/* `boot/hal.s`. The same two instructions the Slopium driver is written over. */
extern void slop_outb(uint16_t port, uint8_t value);
extern uint8_t slop_inb(uint16_t port);

#define COM1 0x3F8u
#define COM1_LINE_STATUS (COM1 + 5u)
#define LINE_STATUS_TRANSMIT_EMPTY 0x20u

/* QEMU's `isa-debug-exit`: a write becomes an exit status of `(value << 1) | 1`,
 * so these are 33 and 35 on the outside. They differ so that a kernel that
 * panicked cannot be read as a kernel that finished. */
#define EXIT_PORT 0xF4u
#define EXIT_PANICKED 0x11u

static unsigned char arena[1 << 16];
static size_t used;

void *sl_rt_alloc(uint64_t size) {
	size_t aligned = ((size_t)size + 15u) & ~(size_t)15u;
	if (aligned < (size_t)size || used + aligned > sizeof arena) {
		return NULL;
	}
	void *block = &arena[used];
	used += aligned;
	return block;
}

void sl_rt_free(void *memory) {
	(void)memory;
}

static void serial_put(char byte) {
	while ((slop_inb(COM1_LINE_STATUS) & LINE_STATUS_TRANSMIT_EMPTY) == 0) {
	}
	slop_outb(COM1, (uint8_t)byte);
}

static _Noreturn void leave(uint32_t code) {
	__asm__ volatile("outl %0, %w1" ::"a"(code), "Nd"((uint16_t)EXIT_PORT));
	for (;;) {
		__asm__ volatile("hlt");
	}
}

_Noreturn void sl_rt_abort(void) {
	leave(EXIT_PANICKED);
}

/* The message is the reason this hook exists at all rather than collapsing into
 * `sl_rt_abort` (`D-080`): a kernel with no stderr still has a serial port, and
 * a panic that says nothing is a machine that stopped for no stated reason. */
_Noreturn void sl_rt_panic(const char *message) {
	serial_put('\n');
	serial_put('!');
	serial_put(' ');
	for (const char *cursor = message; *cursor != '\0'; ++cursor) {
		serial_put(*cursor);
	}
	serial_put('\n');
	leave(EXIT_PANICKED);
}
