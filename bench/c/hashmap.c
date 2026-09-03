/* Hash map workload: 10,000 inserts of key->key, then the same 10,000 keys
 * looked up again. Keys come from a three-shift xorshift64 over a fixed
 * seed, reduced modulo one million, so every language walks the identical
 * key sequence and the collision pattern is identical too.
 *
 * The table is an open-addressing array with linear probing, written here
 * because C ships no hash map: it is the plainest table the language
 * normally reaches for. */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define SEED 88172645463325252ULL
#define COUNT 10000
#define KEY_SPACE 1000000ULL
#define SLOTS (1u << 21)
#define MASK (SLOTS - 1)
#define EMPTY INT64_MIN

static uint64_t xs_next(uint64_t *state)
{
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    return *state;
}

static uint64_t key_hash(uint64_t key)
{
    key ^= key >> 33;
    key ^= key << 21;
    key ^= key >> 24;
    return key;
}

int main(void)
{
    int64_t *table = malloc(sizeof(int64_t) * SLOTS);
    for (size_t i = 0; i < SLOTS; i++)
        table[i] = EMPTY;

    uint64_t state = SEED;
    int64_t size = 0;
    for (int i = 0; i < COUNT; i++)
    {
        int64_t key = (int64_t)(xs_next(&state) % KEY_SPACE);
        size_t at = (size_t)key_hash((uint64_t)key) & MASK;
        while (table[at] != EMPTY && table[at] != key)
            at = (at + 1) & MASK;
        if (table[at] == EMPTY)
        {
            table[at] = key;
            size++;
        }
    }

    int64_t found = 0;
    int64_t sum = 0;
    state = SEED;
    for (int i = 0; i < COUNT; i++)
    {
        int64_t key = (int64_t)(xs_next(&state) % KEY_SPACE);
        size_t at = (size_t)key_hash((uint64_t)key) & MASK;
        while (table[at] != EMPTY && table[at] != key)
            at = (at + 1) & MASK;
        if (table[at] == key)
        {
            found++;
            sum += key;
        }
    }

    printf("%lld\n%lld\n%lld\n", (long long)size, (long long)found, (long long)sum);
    free(table);
    return 0;
}
