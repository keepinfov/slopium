#ifndef STORAGE_STUDY_MODELS_H
#define STORAGE_STUDY_MODELS_H
#include <stddef.h>
#include <stdint.h>
typedef struct StudyStorage StudyStorage;
StudyStorage *study_new(void);
StudyStorage *study_clone(const StudyStorage *);
void study_drop(StudyStorage *);
void study_byte(StudyStorage *, unsigned char);
void study_append(StudyStorage *, const unsigned char *, size_t);
unsigned char study_read(const StudyStorage *, size_t);
size_t study_size(const StudyStorage *);
size_t study_capacity_bytes(const StudyStorage *);
unsigned char *study_build(StudyStorage *);
#endif
