#include <assert.h>
#include <stdint.h>
static uint64_t pushes, packs;
typedef struct SlList SlList;
void __real_sl_rt_list_push(SlList *, const void *);
uint64_t __real_sl_rt_builder_pack_words(uint64_t *, int64_t, uint64_t,
                                       int64_t, const char *, int64_t);
void __wrap_sl_rt_list_push(SlList *list, const void *element) {
    ++pushes;
    __real_sl_rt_list_push(list, element);
}
uint64_t __wrap_sl_rt_builder_pack_words(uint64_t *words, int64_t count,
                                       uint64_t tail, int64_t used,
                                       const char *text, int64_t length) {
    ++packs;
    return __real_sl_rt_builder_pack_words(words, count, tail, used, text, length);
}
extern int64_t answer(void) __asm__("sl_fn_6d61696e3a616e73776572");
int main(void) {
    assert(answer() == 1048577);
    assert(pushes == 131072);
    assert(packs == 256);
    return 0;
}
