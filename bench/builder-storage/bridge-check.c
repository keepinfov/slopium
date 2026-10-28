#include <assert.h>
#include <stdint.h>
#include <string.h>
typedef struct SlString { uint64_t len, cap; char *ptr; } SlString;
uint64_t study_pack(uint64_t *, int64_t, uint64_t, int64_t, const char *, int64_t);
SlString *study_finish(const uint64_t *, int64_t, uint64_t, int64_t);
void sl_rt_string_drop(SlString *);
int main(int argc, char **argv) {
    uint64_t words[2] = {0, UINT64_C(0x1122334455667788)};
    if (argc == 2) {
        if (!strcmp(argv[1],"short")) study_pack(words,0,0,0,"12345678",8);
        else if (!strcmp(argv[1],"negative")) study_pack(words,0,0,-1,"a",1);
        else if (!strcmp(argv[1],"overflow")) study_pack(words,0,0,INT64_MAX,"a",1);
        else if (!strcmp(argv[1],"tail")) study_pack(words,0,256,1,"a",1);
        else if (!strcmp(argv[1],"null")) study_pack(NULL,1,0,0,"12345678",8);
        else if (!strcmp(argv[1],"finish")) study_finish(words,1,0,0);
        else return 2;
        return 3;
    }
    assert(study_pack(NULL,0,0,0,NULL,0)==0);
    uint64_t tail = study_pack(words,1,UINT64_C(0x41414141414141),7,"\xff\0\x80",3);
    assert(words[0]==UINT64_C(0xff41414141414141));
    assert(words[1]==UINT64_C(0x1122334455667788) && tail==UINT64_C(0x8000));
    SlString *text = study_finish(words,1,tail,10);
    assert(text->len==10 && text->cap==11 && text->ptr[10]==0 && !memcmp(text->ptr,"AAAAAAA\xff\0\x80",10));
    sl_rt_string_drop(text);
    text=study_finish(NULL,0,0,0);
    assert(text->len==0 && text->cap==1 && text->ptr[0]==0); sl_rt_string_drop(text);
    return 0;
}
