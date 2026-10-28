#include <assert.h>
#include <stdint.h>
#include <string.h>
uint64_t sl_rt_builder_pack_words(uint64_t *, int64_t, uint64_t, int64_t,
                                 const char *, int64_t);

static unsigned char prefix(int64_t i) { return (unsigned char)(i * 73 + 19); }
int main(int argc, char **argv) {
    if (argc > 1) {
        const char *failure = argv[1];
        uint64_t *bad_words = (uint64_t *)(uintptr_t)1;
        const char *bad_text = (const char *)(uintptr_t)1;
        if (!strcmp(failure, "negative-used"))
            sl_rt_builder_pack_words(bad_words, 0, 0, -1, bad_text, 0);
        else if (!strcmp(failure, "negative-length"))
            sl_rt_builder_pack_words(bad_words, 0, 0, 0, bad_text, INT64_MIN);
        else if (!strcmp(failure, "overflow"))
            sl_rt_builder_pack_words(bad_words, 0, 0, INT64_MAX, bad_text, 1);
        else if (!strcmp(failure, "negative-count"))
            sl_rt_builder_pack_words(bad_words, -1, 0, 0, bad_text, 0);
        else if (!strcmp(failure, "short-count"))
            sl_rt_builder_pack_words(bad_words, 0, 0, 0, bad_text, 8);
        else if (!strcmp(failure, "long-count"))
            sl_rt_builder_pack_words(bad_words, 2, 0, 0, bad_text, 8);
        else if (!strcmp(failure, "null-words"))
            sl_rt_builder_pack_words(NULL, 1, 0, 8, bad_text, 0);
        else if (!strcmp(failure, "null-text"))
            sl_rt_builder_pack_words(NULL, 0, 0, 0, NULL, 1);
        else if (!strcmp(failure, "tail"))
            sl_rt_builder_pack_words(bad_words, 0, 256, 1, bad_text, 0);
        else if (!strcmp(failure, "full-tail"))
            sl_rt_builder_pack_words(bad_words, 1, 1, 8, bad_text, 0);
        return 1;
    }
    assert(sl_rt_builder_pack_words(NULL, 0, 0, 0, NULL, 0) == 0);
    assert(sl_rt_builder_pack_words(NULL, 0, 0x80, 1, NULL, 0) == 0x80);
    unsigned char source[4097], snapshot[4097];
    for (size_t i = 0; i < sizeof source; ++i) source[i] = (unsigned char)i;
    memcpy(snapshot, source, sizeof source);
    const int64_t lengths[] = {0, 1, 7, 8, 9, 31, 32, 33, 255, 256, 257,
                               4095, 4096, 4097};
    const uint64_t guard = UINT64_C(0xdeadbeef5a5aa5a5);
    for (int64_t used = 0; used < 80; ++used) {
        for (size_t n = 0; n < sizeof lengths / sizeof *lengths; ++n) {
            int64_t length = lengths[n], count = (used + length) / 8;
            uint64_t allocation[525], tail = 0;
            uint64_t *words = allocation + 1;
            for (size_t i = 0; i < sizeof allocation / sizeof *allocation; ++i)
                allocation[i] = guard;
            for (int64_t i = 0; i < count; ++i) words[i] = 0;
            for (int64_t i = 0; i < used; ++i) {
                if (i / 8 < used / 8)
                    words[i / 8] |= (uint64_t)prefix(i) << ((i & 7) * 8);
                else tail |= (uint64_t)prefix(i) << ((i & 7) * 8);
            }
            if (count > used / 8) words[used / 8] = tail;
            tail = sl_rt_builder_pack_words(words, count, tail, used,
                                            (const char *)source, length);
            assert(allocation[0] == guard);
            for (size_t i = (size_t)count + 1;
                 i < sizeof allocation / sizeof *allocation; ++i)
                assert(allocation[i] == guard);
            for (int64_t i = 0; i < used + length; ++i) {
                uint64_t word = i / 8 < count ? words[i / 8] : tail;
                unsigned char expected = i < used ? prefix(i) : source[i - used];
                assert((unsigned char)(word >> ((i & 7) * 8)) == expected);
            }
            assert((tail >> (((used + length) & 7) * 8)) == 0);
            assert(!memcmp(source, snapshot, sizeof source));
        }
    }
    return 0;
}
