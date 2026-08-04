/* The half of the runtime that needs a C library.
 *
 * It defines the four symbols `slop_rt_core.c` calls and does not define —
 * `sl_rt_alloc`, `sl_rt_free`, `sl_rt_abort`, `sl_rt_panic` (`D-066`, `D-080`)
 * — and adds what only a hosted program can have: stdio, `argv` and `getenv`.
 *
 * `SlString` is opaque here. This file makes strings only by calling
 * `sl_rt_string_new`, so the layout is core's to know and nobody has to keep
 * two copies of it agreeing. */

#include <ctype.h>
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct SlString SlString;

SlString *sl_rt_string_new(const char *bytes, uint64_t len);

static int32_t sl_argc = 0;
static char **sl_argv = NULL;

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

void sl_rt_println_i32(int32_t value) {
    printf("%d\n", value);
}

void sl_rt_print_i32(int32_t value) {
    printf("%d", value);
}

void sl_rt_println_i64(int64_t value) {
    printf("%ld\n", (long)value);
}

void sl_rt_print_i64(int64_t value) {
    printf("%ld", (long)value);
}

int64_t sl_rt_read_i64(void) {
    char buffer[256];
    if (fgets(buffer, sizeof(buffer), stdin) == NULL) {
        RT_FAIL("expected an integer on stdin");
    }

    errno = 0;
    char *end = NULL;
    long long value = strtoll(buffer, &end, 10);
    if (errno != 0 || end == buffer) {
        RT_FAIL("invalid i64 on stdin");
    }
    while (*end != '\0' && isspace((unsigned char)*end)) {
        end += 1;
    }
    if (*end != '\0') {
        RT_FAIL("invalid trailing data after i64");
    }
    return (int64_t)value;
}

SlString *sl_rt_read_line(void) {
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
    if (value == EOF && length == 0) {
        free(buffer);
        RT_FAIL("expected a line on stdin");
    }
    if (length > 0 && buffer[length - 1] == '\r') {
        length -= 1;
    }
    SlString *line = sl_rt_string_new(buffer, length);
    free(buffer);
    return line;
}

int64_t sl_rt_parse_i64(const char *text, int64_t len) {
    const char *cursor = text;
    const char *limit = text + len;
    while (cursor < limit && isspace((unsigned char)*cursor)) {
        cursor += 1;
    }
    errno = 0;
    char *end = NULL;
    long long value = strtoll(cursor, &end, 10);
    if (errno == ERANGE || end == cursor) {
        RT_FAIL("invalid i64");
    }
    while (end < limit && isspace((unsigned char)*end)) {
        end += 1;
    }
    if (end != limit) {
        RT_FAIL("invalid trailing data after i64");
    }
    return (int64_t)value;
}

SlString *sl_rt_env(const char *name, int64_t len) {
    // Every other string operation is length-based, but getenv stops at the
    // first NUL. Reject the mismatch instead of silently looking up a prefix.
    if (memchr(name, '\0', (size_t)len) != NULL) {
        RT_FAIL("environment variable name contains a NUL byte");
    }
    const char *value = getenv(name);
    if (value == NULL) {
        RT_FAIL("required environment variable is not set");
    }
    return sl_rt_string_new(value, (uint64_t)strlen(value));
}

void sl_rt_args_init(int32_t argc, char **argv) {
    sl_argc = argc;
    sl_argv = argv;
}

int64_t sl_rt_args_len(void) {
    return sl_argc > 0 ? (int64_t)sl_argc - 1 : 0;
}

SlString *sl_rt_arg(int64_t index) {
    if (index < 0 || index >= sl_rt_args_len()) {
        RT_FAIL("process argument index out of bounds");
    }
    const char *value = sl_argv[index + 1];
    return sl_rt_string_new(value, (uint64_t)strlen(value));
}

int32_t sl_rt_test_result(const char *name, int32_t passed) {
    printf("test %s ... %s\n", name, passed ? "ok" : "FAILED");
    return passed ? 0 : 1;
}
