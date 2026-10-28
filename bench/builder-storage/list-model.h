#ifndef STORAGE_STUDY_LIST_H
#define STORAGE_STUDY_LIST_H
#include <stddef.h>
#include <stdint.h>
/* Isolated model of a dynamic element-width list, not a runtime ABI change. */
typedef struct StudyList {
    size_t len, cap, width;
    unsigned char *data;
    void (*drop_element)(void *);
    uint64_t (*clone_element)(uint64_t);
} StudyList;
void study_list_push(StudyList *, const void *);
void study_list_drop(StudyList *);
StudyList study_list_clone(const StudyList *);
#endif
