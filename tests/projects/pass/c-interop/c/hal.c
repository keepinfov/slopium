/* The C half of the `c-interop` fixture: one function per shape the FFI
 * vocabulary allows, so the fixture exercises the whole of `D-065` and not just
 * the easy scalar case.
 *
 * Nothing here is Slopium-aware except `sl_rt_string_new`, which is how C hands
 * back a `String` the caller then owns. */

#include <stdint.h>
#include <string.h>

typedef struct {
    uint64_t len;
    uint64_t cap;
    char *ptr;
} SlString;

SlString *sl_rt_string_new(const char *bytes, uint64_t len);

/* Ten integers: six in registers, four on the stack. */
int64_t hal_sum_ten(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                    int64_t f, int64_t g, int64_t h, int64_t i, int64_t j) {
    return a + b + c + d + e + f + g + h + i + j;
}

/* Ten doubles: eight in SSE registers, two on the stack. */
double hal_sum_ten_doubles(double a, double b, double c, double d, double e,
                           double f, double g, double h, double i, double j) {
    return a + b + c + d + e + f + g + h + i + j;
}

/* A narrow return, whose upper half C leaves undefined. */
int32_t hal_narrow(int32_t value) { return value * 2; }

int hal_is_positive(int64_t value) { return value > 0; }

/* A borrowed `String` arrives as the pointer alone: it is NUL-terminated. */
int64_t hal_strlen(const char *text) { return (int64_t)strlen(text); }

/* A borrowed slice arrives as a pointer and a length, in that order. */
int64_t hal_slice_sum(const int64_t *values, int64_t len) {
    int64_t total = 0;
    for (int64_t index = 0; index < len; index++) {
        total += values[index];
    }
    return total;
}

SlString *hal_greeting(void) { return sl_rt_string_new("hello from C", 12); }

/* A raw pointer is C's `T *`, which is the one spelling in this vocabulary
 * that needs no agreeing about (`D-067`). It crosses in both directions: out
 * as the address of a buffer, and back in as something to read. */
static uint8_t HAL_BUFFER[8];

uint8_t *hal_buffer(void) {
    memset(HAL_BUFFER, 0, sizeof HAL_BUFFER);
    return HAL_BUFFER;
}

int64_t hal_peek(const uint8_t *at) { return (int64_t)*at; }


/* --- What `D-124` opened: C writes -------------------------------------- */

/* A buffer the caller owns, borrowed exclusively, arriving as a pointer and a
 * count. Every element is a machine word whatever the element type is, so this
 * writes `int64_t`s and a byte-at-a-time API needs a shim of its own. C may
 * fill the elements it was given and may not resize the collection. */
void hal_fill(int64_t *values, int64_t len) {
    for (int64_t index = 0; index < len; index++) {
        values[index] = (index + 1) * 10;
    }
}

/* An out-parameter. The slot is a whole machine word, which is why the
 * vocabulary stops at the word-width scalars: `int32_t *` here would leave the
 * upper half of the caller's slot holding whatever was in it. */
int64_t hal_divmod(int64_t value, int64_t by, int64_t *rest) {
    *rest = value % by;
    return value / by;
}

double hal_fraction(double value, double *whole) {
    double units = (double)(int64_t)value;
    *whole = units;
    return value - units;
}

/* A function pointer: C calls back into Slopium, with the Slopium calling
 * convention, which is System V with every argument one word wide. */
int64_t hal_apply_twice(int64_t (*step)(int64_t), int64_t value) {
    return step(step(value));
}
