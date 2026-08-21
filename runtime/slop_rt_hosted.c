/* The half of the runtime that needs a C library.
 *
 * It defines the four symbols `slop_rt_core.c` calls and does not define —
 * `sl_rt_alloc`, `sl_rt_free`, `sl_rt_abort`, `sl_rt_panic` (`D-066`, `D-080`)
 * — and adds what only a hosted program can have: stdio, `argv` and `getenv`.
 *
 * `SlString` is opaque here. This file makes strings only by calling
 * `sl_rt_string_new`, so the layout is core's to know and nobody has to keep
 * two copies of it agreeing. */

#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/random.h>
#include <sys/types.h>
#include <time.h>

typedef struct SlString SlString;

SlString *sl_rt_string_new(const char *bytes, uint64_t len);

static int32_t sl_argc = 0;
static char **sl_argv = NULL;

/* The status slot (`D-085`). Zero is success, a positive value is an `errno`,
 * and `-1` is end of input. Every entry point below that can fail clears it on
 * the way in and sets it on the way out; the library reads it in the form
 * immediately after the call and turns it into an `Option` or a `Result`. */
static int64_t sl_last_error = 0;

int64_t sl_rt_last_error(void) {
    return sl_last_error;
}

/* A message-less abort, for `panic = "abort"` builds. The generated
 * trampolines call this instead of `sl_rt_panic`, and both halves' error paths
 * route through it (see `RT_FAIL`), so a stripped-down binary carries no error
 * strings and does not pull in `fprintf`. */
_Noreturn void sl_rt_abort(void) {
    exit(101);
}

_Noreturn void sl_rt_panic(const char *message) {
    fprintf(stderr, "slopium runtime error: %s\n", message);
    exit(101);
}

#ifdef SLOPIUM_PANIC_ABORT
#define RT_FAIL(message) sl_rt_abort()
#else
#define RT_FAIL(message) sl_rt_panic(message)
#endif

/* The allocator hooks are raw: they may fail, and core turns a failure into a
 * message because core is the half that knows what it was building. */
void *sl_rt_alloc(uint64_t size) {
    return malloc((size_t)size);
}

void sl_rt_free(void *memory) {
    free(memory);
}

/* The library calls these across the FFI, where a `(& String)` arrives as a
 * `const char *` and the length has to travel beside it: a Slopium string may
 * contain a NUL byte, and stopping at the first one would print less than the
 * caller asked for without saying so (`D-079`). */
void sl_rt_println_bytes(const char *bytes, int64_t len) {
    fwrite(bytes, 1, (size_t)len, stdout);
    fputc('\n', stdout);
}

void sl_rt_print_bytes(const char *bytes, int64_t len) {
    fwrite(bytes, 1, (size_t)len, stdout);
}

SlString *sl_rt_read_line(void) {
    sl_last_error = 0;
    uint64_t length = 0;
    uint64_t capacity = 128;
    char *buffer = malloc((size_t)capacity);
    if (buffer == NULL) {
        RT_FAIL("allocation failed");
    }
    int value = 0;
    while ((value = fgetc(stdin)) != EOF && value != '\n') {
        if (length + 1 >= capacity) {
            if (capacity > (uint64_t)SIZE_MAX / 2) {
                free(buffer);
                RT_FAIL("line is too long");
            }
            capacity *= 2;
            void *next = realloc(buffer, (size_t)capacity);
            if (next == NULL) {
                free(buffer);
                RT_FAIL("allocation failed");
            }
            buffer = next;
        }
        buffer[length++] = (char)value;
    }
    /* End of input is a value, not a failure: the library hands back `None`
     * and the program decides (`D-087`). An empty line is a `Some` of an empty
     * string, which is why the slot and not the length is what says so. */
    if (value == EOF && length == 0) {
        free(buffer);
        sl_last_error = -1;
        return sl_rt_string_new("", 0);
    }
    if (length > 0 && buffer[length - 1] == '\r') {
        length -= 1;
    }
    SlString *line = sl_rt_string_new(buffer, length);
    free(buffer);
    return line;
}

SlString *sl_rt_env(const char *name, int64_t len) {
    sl_last_error = 0;
    // Every other string operation is length-based, but getenv stops at the
    // first NUL. A name that contains one cannot name a variable, so this is
    // the same answer as a name that is not set, and not a separate refusal.
    if (memchr(name, '\0', (size_t)len) != NULL) {
        sl_last_error = EINVAL;
        return sl_rt_string_new("", 0);
    }
    const char *value = getenv(name);
    if (value == NULL) {
        sl_last_error = ENOENT;
        return sl_rt_string_new("", 0);
    }
    return sl_rt_string_new(value, (uint64_t)strlen(value));
}

/* A path crosses as a pointer and a length like every other string, and the C
 * calls below take a NUL-terminated one. A path containing a NUL byte names no
 * file, so it is `EINVAL` here rather than a silent truncation (`D-079`). */
static int sl_path_is_usable(const char *path, int64_t len) {
    if (len < 0 || memchr(path, '\0', (size_t)len) != NULL) {
        sl_last_error = EINVAL;
        return 0;
    }
    return 1;
}

SlString *sl_rt_fs_read(const char *path, int64_t len) {
    sl_last_error = 0;
    if (!sl_path_is_usable(path, len)) {
        return sl_rt_string_new("", 0);
    }
    FILE *file = fopen(path, "rb");
    if (file == NULL) {
        sl_last_error = errno;
        return sl_rt_string_new("", 0);
    }

    uint64_t length = 0;
    uint64_t capacity = 1024;
    char *buffer = malloc((size_t)capacity);
    if (buffer == NULL) {
        fclose(file);
        RT_FAIL("allocation failed");
    }
    for (;;) {
        size_t read = fread(buffer + length, 1, (size_t)(capacity - length), file);
        length += (uint64_t)read;
        if (length < capacity) {
            break;
        }
        if (capacity > (uint64_t)SIZE_MAX / 2) {
            free(buffer);
            fclose(file);
            RT_FAIL("file is too large");
        }
        capacity *= 2;
        char *next = realloc(buffer, (size_t)capacity);
        if (next == NULL) {
            free(buffer);
            fclose(file);
            RT_FAIL("allocation failed");
        }
        buffer = next;
    }
    if (ferror(file)) {
        sl_last_error = errno;
        free(buffer);
        fclose(file);
        return sl_rt_string_new("", 0);
    }
    fclose(file);
    SlString *contents = sl_rt_string_new(buffer, length);
    free(buffer);
    return contents;
}

int64_t sl_rt_fs_write(const char *path, int64_t path_len, const char *data, int64_t data_len) {
    sl_last_error = 0;
    if (!sl_path_is_usable(path, path_len) || data_len < 0) {
        sl_last_error = EINVAL;
        return -1;
    }
    FILE *file = fopen(path, "wb");
    if (file == NULL) {
        sl_last_error = errno;
        return -1;
    }
    size_t written = fwrite(data, 1, (size_t)data_len, file);
    if (written != (size_t)data_len || fclose(file) != 0) {
        sl_last_error = errno;
        return -1;
    }
    return data_len;
}

int64_t sl_rt_fs_exists(const char *path, int64_t len) {
    sl_last_error = 0;
    if (!sl_path_is_usable(path, len)) {
        return 0;
    }
    FILE *file = fopen(path, "rb");
    if (file == NULL) {
        return 0;
    }
    fclose(file);
    return 1;
}

int64_t sl_rt_fs_remove(const char *path, int64_t len) {
    sl_last_error = 0;
    if (!sl_path_is_usable(path, len)) {
        return -1;
    }
    if (remove(path) != 0) {
        sl_last_error = errno;
        return -1;
    }
    return 0;
}

_Noreturn void sl_rt_exit(int64_t code) {
    exit((int)code);
}

void sl_rt_args_init(int32_t argc, char **argv) {
    sl_argc = argc;
    sl_argv = argv;
}

int64_t sl_rt_args_len(void) {
    return sl_argc > 0 ? (int64_t)sl_argc - 1 : 0;
}

/* The library checks the index against `sl_rt_args_len` before calling, so
 * this refusal is a backstop for an `extern` written by hand, not the path an
 * out-of-range index takes in a Slopium program (`D-087`). */
SlString *sl_rt_arg(int64_t index) {
    if (index < 0 || index >= sl_rt_args_len()) {
        RT_FAIL("process argument index out of bounds");
    }
    const char *value = sl_argv[index + 1];
    return sl_rt_string_new(value, (uint64_t)strlen(value));
}

/* What a failing test compared, left by `std:test` and printed beside the
 * verdict (`D-130`). One slot is enough because the harness runs one test at a
 * time and clears the note as it reports, and the copy is bounded because a
 * note is a diagnostic rather than a value the program computes with. */
static char sl_test_note[192];
static int sl_test_noted;

void sl_rt_test_note(const char *message) {
    size_t length = strlen(message);
    if (length >= sizeof sl_test_note) {
        length = sizeof sl_test_note - 1;
    }
    memcpy(sl_test_note, message, length);
    sl_test_note[length] = '\0';
    sl_test_noted = 1;
}

int32_t sl_rt_test_result(const char *name, int32_t passed) {
    if (!passed && sl_test_noted) {
        printf("test %s ... FAILED: %s\n", name, sl_test_note);
    } else {
        printf("test %s ... %s\n", name, passed ? "ok" : "FAILED");
    }
    sl_test_noted = 0;
    return passed ? 0 : 1;
}

/* A clock and entropy, both of which belong to the operating system, so both
 * are here and not in `slop_rt_core.c`.
 *
 * `struct timespec` is not in the `extern` vocabulary (`D-065`), so this is
 * where a reading is flattened into one `int64_t` of nanoseconds — which holds
 * a monotonic reading for as long as a machine stays up and a wall-clock one
 * until the year 2262 (`D-147`). */
static int64_t sl_clock_nanos(clockid_t clock) {
    sl_last_error = 0;
    struct timespec now;
    if (clock_gettime(clock, &now) != 0) {
        sl_last_error = errno;
        return 0;
    }
    return (int64_t)now.tv_sec * 1000000000 + (int64_t)now.tv_nsec;
}

int64_t sl_rt_time_monotonic(void) {
    return sl_clock_nanos(CLOCK_MONOTONIC);
}

int64_t sl_rt_time_realtime(void) {
    return sl_clock_nanos(CLOCK_REALTIME);
}

/* `/dev/urandom` is what a kernel without `getrandom` still offers, and what a
 * `getrandom` refused for any reason other than a signal falls back to. */
static int sl_random_from_urandom(unsigned char *out, size_t want) {
    FILE *source = fopen("/dev/urandom", "rb");
    if (source == NULL) {
        return 0;
    }
    size_t got = fread(out, 1, want, source);
    fclose(source);
    return got == want;
}

/* Fills a `(&mut (List u8))` the caller already sized: C is handed the
 * elements and their count and may not resize the collection (`D-124`). One
 * machine word per byte, because that is what a list of any integer type is
 * (`D-107`). Returns how many bytes were written, which the library compares
 * against what it asked for. */
int64_t sl_rt_random_bytes(int64_t *elements, int64_t count) {
    sl_last_error = 0;
    if (count < 0) {
        sl_last_error = EINVAL;
        return 0;
    }
    unsigned char chunk[256];
    int64_t filled = 0;
    while (filled < count) {
        size_t want = (size_t)(count - filled);
        if (want > sizeof chunk) {
            want = sizeof chunk;
        }
        ssize_t got = getrandom(chunk, want, 0);
        if (got < 0) {
            if (errno == EINTR) {
                continue;
            }
            if (!sl_random_from_urandom(chunk, want)) {
                sl_last_error = errno;
                return filled;
            }
            got = (ssize_t)want;
        }
        for (ssize_t index = 0; index < got; index += 1) {
            elements[filled + index] = (int64_t)chunk[index];
        }
        filled += (int64_t)got;
    }
    return filled;
}
