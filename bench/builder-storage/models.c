/* Native C cost models only. STORAGE=0: packed word list with a scalar tail;
 * 1: dedicated owning byte buffer; 2: generic list with element width one.
 * Models 1/2 do not imply that the Slopium compiler supports those owners. */
#include "models.h"
#include "list-model.h"
#include <stdlib.h>
#include <string.h>
#ifndef STORAGE
#error STORAGE is required
#endif

struct StudyStorage { StudyList list; uint64_t tail; size_t used; };
StudyStorage *study_new(void) {
    StudyStorage *out = calloc(1, sizeof *out);
    if (!out) abort();
    out->list.width = STORAGE == 0 ? 8 : 1;
    return out;
}
void study_drop(StudyStorage *out) {
    if (out) { study_list_drop(&out->list); free(out); }
}
StudyStorage *study_clone(const StudyStorage *source) {
    StudyStorage *copy = study_new();
    copy->list = study_list_clone(&source->list);
    copy->tail = source->tail;
    copy->used = source->used;
    return copy;
}
#if STORAGE == 1
static void reserve(StudyStorage *out, size_t extra) {
    if (extra > SIZE_MAX - out->used) abort();
    size_t needed = out->used + extra;
    if (needed <= out->list.cap) return;
    size_t next = out->list.cap ? out->list.cap : 4;
    while (next < needed) {
        if (next > SIZE_MAX / 2) abort();
        next *= 2;
    }
    unsigned char *data = malloc(next);
    if (!data) abort();
    if (out->used) memcpy(data, out->list.data, out->used);
    free(out->list.data);
    out->list.data = data;
    out->list.cap = next;
}
#endif
void study_byte(StudyStorage *out, unsigned char byte) {
    if (out->used == SIZE_MAX) abort();
#if STORAGE == 0
    out->tail |= (uint64_t)byte << ((out->used & 7) * 8);
    if ((out->used & 7) == 7) {
        study_list_push(&out->list, &out->tail);
        out->tail = 0;
    }
#elif STORAGE == 1
    reserve(out, 1);
    out->list.data[out->used] = byte;
    ++out->list.len;
#else
    study_list_push(&out->list, &byte);
#endif
    ++out->used;
}
void study_append(StudyStorage *out, const unsigned char *text, size_t length) {
    if (length > SIZE_MAX - out->used || (length && !text)) abort();
#if STORAGE == 1
    reserve(out, length);
    if (length) memcpy(out->list.data + out->used, text, length);
    out->used += length;
    out->list.len = out->used;
#elif STORAGE == 0
    /* Match the Slopium prototype's word prefill and one borrowed pack call. */
    size_t complete = (out->used + length) / 8;
    uint64_t zero = 0;
    while (out->list.len < complete) study_list_push(&out->list, &zero);
    uint64_t *words = (uint64_t *)out->list.data;
    for (size_t i = 0; i < length; ++i) {
        out->tail |= (uint64_t)text[i] << ((out->used & 7) * 8);
        if ((out->used & 7) == 7) { words[out->used / 8] = out->tail; out->tail = 0; }
        ++out->used;
    }
#else
    for (size_t i = 0; i < length; ++i) study_byte(out, text[i]);
#endif
}
unsigned char study_read(const StudyStorage *out, size_t index) {
    if (index >= out->used) abort();
#if STORAGE == 0
    uint64_t word = index / 8 < out->list.len ?
        ((const uint64_t *)out->list.data)[index / 8] : out->tail;
    return (unsigned char)(word >> ((index & 7) * 8));
#else
    return out->list.data[index * out->list.width];
#endif
}
size_t study_size(const StudyStorage *out) { return out->used; }
size_t study_capacity_bytes(const StudyStorage *out) {
    return out->list.cap * out->list.width + (STORAGE == 0 ? 8 : 0);
}
unsigned char *study_build(StudyStorage *out) {
    if (out->used == SIZE_MAX) abort();
    unsigned char *text = malloc(out->used + 1);
    if (!text) abort();
#if STORAGE == 0
    for (size_t i = 0; i < out->used; ++i) text[i] = study_read(out, i);
#else
    if (out->used) memcpy(text, out->list.data, out->used);
#endif
    text[out->used] = 0;
    study_drop(out);
    return text;
}
