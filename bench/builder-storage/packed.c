/* Research bridge over the existing word-list ABI. No production symbol or
 * list layout changes; a borrow ends before any later List growth. */
#include <stddef.h>
#include <stdint.h>

typedef struct SlString { uint64_t len, cap; char *ptr; } SlString;
void *sl_rt_alloc(uint64_t);
_Noreturn void sl_rt_panic(const char *);

uint64_t study_pack(uint64_t *words, int64_t count, uint64_t tail,
                    int64_t used, const char *text, int64_t length) {
    if (used < 0 || length < 0 || used > INT64_MAX - length ||
        count != (used + length) / 8 || (count && !words) || (length && !text) ||
        (tail >> (((uint64_t)used & 7) * 8)))
        sl_rt_panic("storage study: packed append bounds");
    uint64_t position = (uint64_t)used;
    for (int64_t i = 0; i < length; ++i) {
        tail |= (uint64_t)(unsigned char)text[i] << ((position & 7) * 8);
        if ((position & 7) == 7) {
            words[position / 8] = tail;
            tail = 0;
        }
        ++position;
    }
    return tail;
}

SlString *study_finish(const uint64_t *words, int64_t count,
                       uint64_t tail, int64_t used) {
    if (used < 0 || count != used / 8 || (count && !words) ||
        (uint64_t)used >= (uint64_t)SIZE_MAX ||
        (tail >> (((uint64_t)used & 7) * 8)))
        sl_rt_panic("storage study: packed finish bounds");
    SlString *text = sl_rt_alloc(sizeof *text);
    if (!text) sl_rt_panic("storage study: allocation refused");
    text->len = (uint64_t)used;
    text->cap = (uint64_t)used + 1;
    text->ptr = sl_rt_alloc((uint64_t)used + 1);
    if (!text->ptr) sl_rt_panic("storage study: allocation refused");
    for (int64_t i = 0; i < used; ++i) {
        uint64_t word = i / 8 < count ? words[i / 8] : tail;
        text->ptr[i] = (char)(word >> ((i & 7) * 8));
    }
    text->ptr[used] = '\0';
    return text;
}
