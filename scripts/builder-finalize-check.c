#include <assert.h>
#include <stdint.h>
#include <string.h>
#include <stddef.h>
#include <stdlib.h>
typedef struct SlString { uint64_t len, cap; char *ptr; } SlString;
SlString *sl_rt_string_from_words(const uint64_t *, int64_t, uint64_t, int64_t);
void sl_rt_string_drop(SlString *);
static unsigned char byte(int64_t index) { return (unsigned char)(index * 73 + 29); }

int main(int argc, char **argv) {
    if (argc > 1) {
        const uint64_t *bad = (const uint64_t *)(uintptr_t)1;
        if (!strcmp(argv[1], "negative-count")) sl_rt_string_from_words(bad, -1, 0, 0);
        else if (!strcmp(argv[1], "negative")) sl_rt_string_from_words(bad, 0, 0, -1);
        else if (!strcmp(argv[1], "short")) sl_rt_string_from_words(bad, 0, 0, 8);
        else if (!strcmp(argv[1], "long")) sl_rt_string_from_words(bad, 1, 0, 0);
        else if (!strcmp(argv[1], "null")) sl_rt_string_from_words(NULL, 1, 0, 8);
        else if (!strcmp(argv[1], "tail")) sl_rt_string_from_words(bad, 0, 256, 1);
        else if (!strcmp(argv[1], "full-tail")) sl_rt_string_from_words(bad, 1, 1, 8);
        return 1;
    }
    for (int64_t used = 0; used <= 4097; ++used) {
        int64_t count = used / 8;
        size_t width = (size_t)count * sizeof(uint64_t);
        uint64_t *words = count ? calloc((size_t)count, sizeof(uint64_t)) : NULL;
        uint64_t *snapshot = count ? malloc(width) : NULL;
        assert(!count || (words && snapshot));
        uint64_t tail = 0;
        for (int64_t i = 0; i < used; ++i) {
            if (i / 8 < count) words[i / 8] |= (uint64_t)byte(i) << ((i & 7) * 8);
            else tail |= (uint64_t)byte(i) << ((i & 7) * 8);
        }
        if (count) memcpy(snapshot, words, width);
        SlString *s = sl_rt_string_from_words(count ? words : NULL, count, tail, used);
        assert(s->len == (uint64_t)used && s->cap == (uint64_t)used + 1);
        assert(s->ptr[used] == '\0');
        for (int64_t i = 0; i < used; ++i) assert((unsigned char)s->ptr[i] == byte(i));
        if (count) assert(!memcmp(words, snapshot, width));
        free(words);
        free(snapshot);
        if (used) {
            assert((unsigned char)s->ptr[0] == byte(0));
            assert((unsigned char)s->ptr[used - 1] == byte(used - 1));
        }
        assert(s->ptr[used] == '\0');
        sl_rt_string_drop(s);
    }
    return 0;
}
