/* String building, incrementally: 50,000 records of "item " + number + '\n'
 * appended to one growing buffer, then the result measured. The buffer is
 * realloc-doubling, which is the plainest growing buffer C normally reaches
 * for.
 *
 * The checksum is the byte length of the result and the sum of its bytes. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define RECORDS 50000

int main(void)
{
    size_t capacity = 4096;
    size_t length = 0;
    char *buffer = malloc(capacity);
    for (long long i = 0; i < RECORDS; i++)
    {
        char piece[64];
        int width = snprintf(piece, sizeof(piece), "item %lld\n", i);
        if (length + (size_t)width > capacity)
        {
            while (length + (size_t)width > capacity)
                capacity *= 2;
            buffer = realloc(buffer, capacity);
        }
        memcpy(buffer + length, piece, (size_t)width);
        length += (size_t)width;
    }
    long long sum = 0;
    for (size_t i = 0; i < length; i++)
        sum += (unsigned char)buffer[i];
    printf("%zu\n%lld\n", length, sum);
    free(buffer);
    return 0;
}
