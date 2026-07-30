#include <ctype.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    uint64_t len;
    uint64_t cap;
    char *ptr;
} SlString;

typedef struct {
    uint64_t len;
    uint64_t cap;
    uint64_t elem_size;
    unsigned char *ptr;
    void (*drop_element)(void *);
    uint64_t (*clone_element)(uint64_t);
} SlList;

typedef struct {
    uint64_t len;
    uint64_t elem_size;
    unsigned char *ptr;
} SlSlice;

static int32_t sl_argc = 0;
static char **sl_argv = NULL;

/* A message-less abort, for `panic = "abort"` builds. The generated
 * trampolines call this instead of `sl_rt_panic`, and the runtime's own error
 * paths route through it (see `RT_FAIL`), so a stripped-down binary carries no
 * error strings and does not pull in `fprintf`. */
_Noreturn void sl_rt_abort(void) {
    exit(101);
}

_Noreturn void sl_rt_panic(const char *message) {
    fprintf(stderr, "slopium runtime error: %s\n", message);
    exit(101);
}

/* The runtime's internal failures. In the default build this is `sl_rt_panic`
 * with a message; under `-DSLOPIUM_PANIC_ABORT` it drops the message at the
 * call site, so the literal never reaches the binary. */
#ifdef SLOPIUM_PANIC_ABORT
#define RT_FAIL(message) sl_rt_abort()
#else
#define RT_FAIL(message) sl_rt_panic(message)
#endif

static void *sl_checked_alloc(uint64_t size) {
    void *memory = malloc((size_t)size);
    if (memory == NULL && size != 0) {
        RT_FAIL("allocation failed");
    }
    return memory;
}

void *sl_rt_alloc(uint64_t size) {
    return sl_checked_alloc(size);
}

void sl_rt_free(void *memory) {
    free(memory);
}

SlString *sl_rt_string_new(const char *bytes, uint64_t len) {
    if (len == UINT64_MAX || len + 1 > (uint64_t)SIZE_MAX) {
        RT_FAIL("string length overflow");
    }
    SlString *string = sl_checked_alloc(sizeof(SlString));
    string->ptr = sl_checked_alloc(len + 1);
    memcpy(string->ptr, bytes, (size_t)len);
    string->ptr[len] = '\0';
    string->len = len;
    string->cap = len + 1;
    return string;
}

SlString *sl_rt_string_clone(const SlString *source) {
    return sl_rt_string_new(source->ptr, source->len);
}

void sl_rt_string_drop(SlString *string) {
    if (string != NULL) {
        free(string->ptr);
        free(string);
    }
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

void sl_rt_list_push(SlList *list, const void *element) {
    if (list->len == list->cap) {
        if (list->cap > UINT64_MAX / 2) {
            RT_FAIL("list capacity overflow");
        }
        uint64_t next_cap = list->cap == 0 ? 4 : list->cap * 2;
        if (list->elem_size != 0 && next_cap > (uint64_t)SIZE_MAX / list->elem_size) {
            RT_FAIL("list capacity overflow");
        }
        void *next = realloc(list->ptr, (size_t)(next_cap * list->elem_size));
        if (next == NULL) {
            RT_FAIL("allocation failed");
        }
        list->ptr = next;
        list->cap = next_cap;
    }
    memcpy(list->ptr + list->len * list->elem_size, element, (size_t)list->elem_size);
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
    memcpy(&value, list->ptr + list->len * list->elem_size, (size_t)list->elem_size);
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
    memcpy(&value, element, (size_t)list->elem_size);
    if (index + 1 < list->len) {
        memmove(element,
                element + list->elem_size,
                (size_t)((list->len - index - 1) * list->elem_size));
    }
    list->len -= 1;
    return value;
}

SlList *sl_rt_list_clone(const SlList *source) {
    SlList *copy =
        sl_rt_list_new(source->elem_size, source->drop_element, source->clone_element);
    for (uint64_t index = 0; index < source->len; index += 1) {
        uint64_t value = 0;
        memcpy(&value,
               source->ptr + index * source->elem_size,
               (size_t)source->elem_size);
        if (source->clone_element != NULL) {
            value = source->clone_element(value);
        }
        sl_rt_list_push(copy, &value);
    }
    return copy;
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
    *copy = *source;
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
    free(slice);
}

void sl_rt_list_drop(SlList *list) {
    if (list != NULL) {
        if (list->drop_element != NULL) {
            for (uint64_t index = 0; index < list->len; index += 1) {
                uint64_t value = 0;
                memcpy(&value,
                       list->ptr + index * list->elem_size,
                       (size_t)list->elem_size);
                list->drop_element((void *)(uintptr_t)value);
            }
        }
        free(list->ptr);
        free(list);
    }
}

void sl_rt_println_string(const SlString *string) {
    fwrite(string->ptr, 1, (size_t)string->len, stdout);
    fputc('\n', stdout);
}

void sl_rt_print_string(const SlString *string) {
    fwrite(string->ptr, 1, (size_t)string->len, stdout);
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
    char *buffer = sl_checked_alloc(capacity);
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

int64_t sl_rt_parse_i64(const SlString *text) {
    const char *cursor = text->ptr;
    const char *limit = text->ptr + text->len;
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

SlString *sl_rt_env(const SlString *name) {
    // Every other string operation is length-based, but getenv stops at the
    // first NUL. Reject the mismatch instead of silently looking up a prefix.
    if (memchr(name->ptr, '\0', (size_t)name->len) != NULL) {
        RT_FAIL("environment variable name contains a NUL byte");
    }
    const char *value = getenv(name->ptr);
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
