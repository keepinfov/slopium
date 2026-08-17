/* The four symbols `slop_rt_core.c` calls and does not define (`D-066`,
 * `D-080`). A freestanding program owes the world exactly these, which is the
 * property `scripts/core-check.sh` asserts with `nm -u`.
 *
 * The allocator is a bump allocator over a static arena and never reuses
 * anything, because a fixture that exits after one answer has nothing to gain
 * from a free list. A kernel supplies a real one; the point is that `core` calls
 * these rather than `malloc`. */

#include <stddef.h>
#include <stdint.h>

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

static _Noreturn void leave(int code) {
	__asm__ volatile("syscall" ::"a"(60), "D"((long)code) : "memory");
	__builtin_unreachable();
}

/* 101 is the status an unrecoverable Slopium error exits with everywhere else,
 * and there is no stderr here to say more on. */
_Noreturn void sl_rt_abort(void) {
	leave(101);
}

_Noreturn void sl_rt_panic(const char *message) {
	(void)message;
	leave(101);
}
