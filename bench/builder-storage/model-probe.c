#define _POSIX_C_SOURCE 200809L
#include "models.h"
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
static uint64_t now(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value)) abort();
    return (uint64_t)value.tv_sec * 1000000000 + (uint64_t)value.tv_nsec;
}
static void integer(StudyStorage *out, uint64_t value) {
    if (value >= 10) integer(out, value / 10);
    study_byte(out, (unsigned char)(48 + value % 10));
}
static void check(void) {
    const unsigned char piece[] = {'a',0,128,255,195,169,'z'};
    StudyStorage *abandoned = study_new();
    study_append(abandoned, piece, sizeof piece);
    study_drop(abandoned);
    for (size_t offset=0; offset<8; ++offset) {
        StudyStorage *out = study_new();
        for (size_t i=0; i<offset; ++i) study_byte(out, '>');
        for (size_t i=0; i<100; ++i) study_append(out, piece, sizeof piece);
        study_append(out, NULL, 0);
        StudyStorage *copy = study_clone(out);
        study_byte(copy, '!');
        size_t width = offset + 100*sizeof piece;
        assert(study_size(out)==width && study_size(copy)==width+1);
        for (size_t i=0; i<width; ++i) {
            unsigned char expected = i<offset ? '>' : piece[(i-offset)%sizeof piece];
            assert(study_read(out,i)==expected && study_read(copy,i)==expected);
        }
        unsigned char *left=study_build(out), *right=study_build(copy);
        assert(!memcmp(left,right,width) && right[width]=='!');
        free(left); free(right);
    }
    StudyStorage *empty=study_new();
    unsigned char *text=study_build(empty); assert(text[0]==0); free(text);
}
int main(int argc, char **argv) {
    if (argc==2 && !strcmp(argv[1],"--check")) { check(); return 0; }
    if (argc!=4) return 2;
    int mode=atoi(argv[1]);
    size_t count=strtoull(argv[2],NULL,10), width=strtoull(argv[3],NULL,10);
    if (width==SIZE_MAX) return 2;
    unsigned char *piece=malloc(width+1); if (!piece) abort(); memset(piece,'a',width);
    uint64_t start=now(); StudyStorage *out=study_new();
    for (size_t i=0; i<count; ++i) {
        if (mode==0) {
            study_append(out,(const unsigned char *)"item ",5);
            integer(out,i); study_byte(out,10);
        } else if (mode==1) study_append(out,piece,width);
        else integer(out,mode==3 ? UINT64_MAX-i : i);
    }
    uint64_t appended=now(); size_t length=study_size(out), capacity=study_capacity_bytes(out);
    uint64_t read_sum=0; for (size_t i=0; i<length; ++i) read_sum+=study_read(out,i);
    uint64_t read=now(); unsigned char *text=study_build(out); uint64_t finalized=now();
    uint64_t sum=0; for (size_t i=0; i<length; ++i) sum+=text[i];
    uint64_t checked=now(), clock_end=now();
    printf("%zu\n%" PRIu64 "\n%" PRIu64 "\n%" PRIu64 "\n%" PRIu64 "\n%" PRIu64 "\n%" PRIu64 "\n%" PRIu64 "\n%zu\n",
           length,sum,read_sum,appended-start,read-appended,finalized-read,
           checked-finalized,clock_end-checked,capacity);
    free(text); free(piece); return 0;
}
