/* The half of the runtime that a freestanding program can have.
 *
 * Strings, lists and slices, and the failure paths the compiler's trampolines
 * branch to. It calls `sl_rt_alloc`, `sl_rt_free`, `sl_rt_abort` and
 * `sl_rt_panic` and defines none of them: a kernel has an allocator and a way
 * to die, what it does not have is libc (`D-066`, `D-080`).
 *
 * Nothing here includes a hosted header, and the byte moves are written out
 * rather than called for, so an object built from this file has no undefined
 * symbol beyond those four. `scripts/core-check.sh` is what says so, and it is
 * the check rather than the flags that has to hold: a compiler is allowed to
 * recognize the loops below and emit the `memcpy` call they were written to
 * avoid, which is why `RUNTIME_CORE_FLAGS` adds `-ffreestanding -fno-builtin`
 * and why the gate reads `nm -u` instead of trusting them. */

/* Both are freestanding headers: the C standard requires an implementation to
 * provide them with no hosted library behind them. */
#include <stddef.h>
#include <stdint.h>

typedef struct SlString {
    uint64_t len;
    uint64_t cap;
    char *ptr;
} SlString;

typedef struct SlList {
    uint64_t len;
    uint64_t cap;
    uint64_t elem_size;
    unsigned char *ptr;
    void (*drop_element)(void *);
    uint64_t (*clone_element)(uint64_t);
} SlList;

typedef struct SlSlice {
    uint64_t len;
    uint64_t elem_size;
    unsigned char *ptr;
} SlSlice;

/* The seam. Hosted defines all four; a freestanding program defines the three
 * it needs, and the fourth only if it was not built with panic-abort. */
void *sl_rt_alloc(uint64_t size);
void sl_rt_free(void *memory);
_Noreturn void sl_rt_abort(void);
_Noreturn void sl_rt_panic(const char *message);

/* The runtime's internal failures. In the default build this is `sl_rt_panic`
 * with a message; under `-DSLOPIUM_PANIC_ABORT` it drops the message at the
 * call site, so the literal never reaches the binary. */
#ifdef SLOPIUM_PANIC_ABORT
#define RT_FAIL(message) sl_rt_abort()
#else
#define RT_FAIL(message) sl_rt_panic(message)
#endif

static void sl_mem_copy(void *destination, const void *source, uint64_t count) {
    unsigned char *out = destination;
    const unsigned char *in = source;
    for (uint64_t index = 0; index < count; index += 1) {
        out[index] = in[index];
    }
}

/* Backwards when the regions overlap the wrong way, which is the case
 * `sl_rt_list_remove` produces every time it closes a gap. */
static void sl_mem_move(void *destination, const void *source, uint64_t count) {
    unsigned char *out = destination;
    const unsigned char *in = source;
    if (out < in) {
        for (uint64_t index = 0; index < count; index += 1) {
            out[index] = in[index];
        }
    } else {
        for (uint64_t index = count; index > 0; index -= 1) {
            out[index - 1] = in[index - 1];
        }
    }
}

/* `sl_rt_alloc` is a hook and may return null; the message for a refusal
 * belongs here, in the half that knows what was being built (`D-080`). */
static void *sl_checked_alloc(uint64_t size) {
    void *memory = sl_rt_alloc(size);
    if (memory == NULL && size != 0) {
        RT_FAIL("allocation failed");
    }
    return memory;
}

SlString *sl_rt_string_new(const char *bytes, uint64_t len) {
    if (len == UINT64_MAX || len + 1 > (uint64_t)SIZE_MAX) {
        RT_FAIL("string length overflow");
    }
    SlString *string = sl_checked_alloc(sizeof(SlString));
    string->ptr = sl_checked_alloc(len + 1);
    sl_mem_copy(string->ptr, bytes, len);
    string->ptr[len] = '\0';
    string->len = len;
    string->cap = len + 1;
    return string;
}

uint64_t sl_rt_string_len(const SlString *string) {
    return string->len;
}

SlString *sl_rt_string_clone(const SlString *source) {
    return sl_rt_string_new(source->ptr, source->len);
}

void sl_rt_string_drop(SlString *string) {
    if (string != NULL) {
        sl_rt_free(string->ptr);
        sl_rt_free(string);
    }
}

/* A function value is a block the compiler lays out as a struct — the code
 * address, this pair of helpers, then one word per capture (`D-101`). The
 * helpers are the ones both backends already generate for every struct, so all
 * that is needed here is the dispatch: the static type says `Fn` and cannot say
 * which closure, because two closures of one type capture different things.
 *
 * A null block is a no-op on the way out, matching `sl_rt_string_drop` and
 * `sl_rt_list_drop`, so a slot the compiler has already dropped and zeroed
 * stays benign. */
typedef void (*SlClosureDrop)(void *closure);
typedef void *(*SlClosureClone)(const void *closure);

void sl_rt_closure_drop(void *closure) {
    if (closure != NULL) {
        ((SlClosureDrop *)closure)[1](closure);
    }
}

void *sl_rt_closure_clone(const void *closure) {
    return ((SlClosureClone *)closure)[2](closure);
}

/* The four the library cannot write for itself (`D-083`). Everything else in
 * `core:string` — formatting, parsing, splitting, trimming — is Slopium over
 * these, because a `(& String)` reaches C as a pointer and a length and there
 * is no other way back into the bytes. */
int64_t sl_rt_string_byte(const char *bytes, int64_t len, int64_t index) {
    if (index < 0 || index >= len) {
        RT_FAIL("string index out of bounds");
    }
    return (int64_t)(unsigned char)bytes[index];
}

SlString *sl_rt_string_slice(const char *bytes, int64_t len, int64_t start, int64_t end) {
    if (start < 0 || end < start || end > len) {
        RT_FAIL("string range out of bounds");
    }
    return sl_rt_string_new(bytes + start, (uint64_t)(end - start));
}

SlString *sl_rt_string_concat(const char *left, int64_t left_len,
                              const char *right, int64_t right_len) {
    if (left_len < 0 || right_len < 0) {
        RT_FAIL("negative string length");
    }
    uint64_t total = (uint64_t)left_len + (uint64_t)right_len;
    if (total == UINT64_MAX || total + 1 > (uint64_t)SIZE_MAX) {
        RT_FAIL("string length overflow");
    }
    SlString *string = sl_checked_alloc(sizeof(SlString));
    string->ptr = sl_checked_alloc(total + 1);
    sl_mem_copy(string->ptr, left, (uint64_t)left_len);
    sl_mem_copy(string->ptr + left_len, right, (uint64_t)right_len);
    string->ptr[total] = '\0';
    string->len = total;
    string->cap = total + 1;
    return string;
}

/* One `i64` per byte, because the library builds its bytes in a `(List i64)`
 * and there is no narrower element type to slice. Everything above 8 bits is
 * dropped, which is what a byte is. */
SlString *sl_rt_string_from_bytes(const int64_t *bytes, uint64_t count) {
    if (count == UINT64_MAX || count + 1 > (uint64_t)SIZE_MAX) {
        RT_FAIL("string length overflow");
    }
    SlString *string = sl_checked_alloc(sizeof(SlString));
    string->ptr = sl_checked_alloc(count + 1);
    for (uint64_t index = 0; index < count; index += 1) {
        string->ptr[index] = (unsigned char)(bytes[index] & 0xff);
    }
    string->ptr[count] = '\0';
    string->len = count;
    string->cap = count + 1;
    return string;
}

SlList *sl_rt_list_new(uint64_t elem_size,
                       void (*drop_element)(void *),
                       uint64_t (*clone_element)(uint64_t)) {
    SlList *list = sl_checked_alloc(sizeof(SlList));
    list->len = 0;
    list->cap = 0;
    list->elem_size = elem_size;
    list->ptr = NULL;
    list->drop_element = drop_element;
    list->clone_element = clone_element;
    return list;
}

/* Grows by allocate, copy and free rather than by `realloc`, which would have
 * been a fifth hook for the sake of a copy this path was making anyway
 * (`D-080`). */
void sl_rt_list_push(SlList *list, const void *element) {
    if (list->len == list->cap) {
        if (list->cap > UINT64_MAX / 2) {
            RT_FAIL("list capacity overflow");
        }
        uint64_t next_cap = list->cap == 0 ? 4 : list->cap * 2;
        if (list->elem_size != 0 && next_cap > (uint64_t)SIZE_MAX / list->elem_size) {
            RT_FAIL("list capacity overflow");
        }
        unsigned char *next = sl_checked_alloc(next_cap * list->elem_size);
        if (list->ptr != NULL) {
            sl_mem_copy(next, list->ptr, list->len * list->elem_size);
            sl_rt_free(list->ptr);
        }
        list->ptr = next;
        list->cap = next_cap;
    }
    sl_mem_copy(list->ptr + list->len * list->elem_size, element, list->elem_size);
    list->len += 1;
}

uint64_t sl_rt_list_len(const SlList *list) {
    return list->len;
}

void *sl_rt_list_get(const SlList *list, uint64_t index) {
    if (index >= list->len) {
        RT_FAIL("list index out of bounds");
    }
    return list->ptr + index * list->elem_size;
}

uint64_t sl_rt_list_pop(SlList *list) {
    if (list->len == 0) {
        RT_FAIL("pop from empty list");
    }
    list->len -= 1;
    uint64_t value = 0;
    sl_mem_copy(&value, list->ptr + list->len * list->elem_size, list->elem_size);
    return value;
}

uint64_t sl_rt_list_try_pop(SlList *list, uint64_t *output) {
    if (list->len == 0) {
        return 0;
    }
    *output = sl_rt_list_pop(list);
    return 1;
}

uint64_t sl_rt_list_remove(SlList *list, uint64_t index) {
    if (index >= list->len) {
        RT_FAIL("list index out of bounds");
    }
    uint64_t value = 0;
    unsigned char *element = list->ptr + index * list->elem_size;
    sl_mem_copy(&value, element, list->elem_size);
    if (index + 1 < list->len) {
        sl_mem_move(element,
                    element + list->elem_size,
                    (list->len - index - 1) * list->elem_size);
    }
    list->len -= 1;
    return value;
}

/* The one write a list did not have. Nothing moves: the new element takes the
 * slot and the old one is handed back, so the caller owns exactly one of them
 * before the call and exactly one after. */
uint64_t sl_rt_list_replace(SlList *list, uint64_t index, const void *element) {
    if (index >= list->len) {
        RT_FAIL("list index out of bounds");
    }
    uint64_t value = 0;
    unsigned char *slot = list->ptr + index * list->elem_size;
    sl_mem_copy(&value, slot, list->elem_size);
    sl_mem_copy(slot, element, list->elem_size);
    return value;
}

SlList *sl_rt_list_clone(const SlList *source) {
    SlList *copy =
        sl_rt_list_new(source->elem_size, source->drop_element, source->clone_element);
    for (uint64_t index = 0; index < source->len; index += 1) {
        uint64_t value = 0;
        sl_mem_copy(&value, source->ptr + index * source->elem_size, source->elem_size);
        if (source->clone_element != NULL) {
            value = source->clone_element(value);
        }
        sl_rt_list_push(copy, &value);
    }
    return copy;
}

void sl_rt_list_drop(SlList *list) {
    if (list != NULL) {
        if (list->drop_element != NULL) {
            for (uint64_t index = 0; index < list->len; index += 1) {
                uint64_t value = 0;
                sl_mem_copy(&value,
                            list->ptr + index * list->elem_size,
                            list->elem_size);
                list->drop_element((void *)(uintptr_t)value);
            }
        }
        sl_rt_free(list->ptr);
        sl_rt_free(list);
    }
}

SlSlice *sl_rt_slice_new(const SlList *source, uint64_t start, uint64_t end) {
    if (start > end || end > source->len) {
        RT_FAIL("slice range out of bounds");
    }
    SlSlice *slice = sl_checked_alloc(sizeof(SlSlice));
    slice->len = end - start;
    slice->elem_size = source->elem_size;
    slice->ptr = source->ptr + start * source->elem_size;
    return slice;
}

SlSlice *sl_rt_slice_clone(const SlSlice *source) {
    SlSlice *copy = sl_checked_alloc(sizeof(SlSlice));
    /* Field by field, not `*copy = *source`: a struct assignment is one of the
     * shapes a compiler is free to turn into a `memcpy` call. */
    copy->len = source->len;
    copy->elem_size = source->elem_size;
    copy->ptr = source->ptr;
    return copy;
}

uint64_t sl_rt_slice_len(const SlSlice *slice) {
    return slice->len;
}

void *sl_rt_slice_get(const SlSlice *slice, uint64_t index) {
    if (index >= slice->len) {
        RT_FAIL("slice index out of bounds");
    }
    return slice->ptr + index * slice->elem_size;
}

void sl_rt_slice_drop(SlSlice *slice) {
    sl_rt_free(slice);
}

/* The two the library cannot write for itself, for the same reason the four
 * string primitives exist: an `f64` is a value the language can compute with
 * and cannot look inside, and every digit of a decimal expansion comes from
 * the sign, exponent and significand. A union is the bit reinterpretation C
 * defines; nothing here rounds, traps, or reads a floating-point flag, so an
 * object built from this file still has no undefined symbol. */
typedef union SlF64Bits {
    double value;
    int64_t bits;
} SlF64Bits;

int64_t sl_rt_f64_bits(double value) {
    SlF64Bits pun;
    pun.value = value;
    return pun.bits;
}

double sl_rt_f64_from_bits(int64_t bits) {
    SlF64Bits pun;
    pun.bits = bits;
    return pun.value;
}
