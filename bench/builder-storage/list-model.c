#include "list-model.h"
#include <stdlib.h>
#include <string.h>

/* Separate translation unit preserves the generic push call boundary. */
void study_list_push(StudyList *list, const void *element) {
    if (list->len == list->cap) {
        if (list->cap > SIZE_MAX / 2) abort();
        size_t next = list->cap ? list->cap * 2 : 4;
        if (!list->width || next > SIZE_MAX / list->width) abort();
        unsigned char *data = malloc(next * list->width);
        if (!data) abort();
        if (list->len) memcpy(data, list->data, list->len * list->width);
        free(list->data);
        list->data = data;
        list->cap = next;
    }
    memcpy(list->data + list->len * list->width, element, list->width);
    ++list->len;
}
void study_list_drop(StudyList *list) {
    if (list->drop_element || list->clone_element) abort();
    free(list->data);
    *list = (StudyList){0};
}
StudyList study_list_clone(const StudyList *source) {
    if (source->drop_element || source->clone_element) abort();
    StudyList copy = {.width = source->width};
    for (size_t i = 0; i < source->len; ++i)
        study_list_push(&copy, source->data + i * source->width);
    return copy;
}
