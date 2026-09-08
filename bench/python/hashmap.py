# Hash map workload: 10,000 inserts of key->key, then the same 10,000 keys
# looked up again. Keys come from a three-shift xorshift64 over a fixed
# seed, reduced modulo one million, so every language walks the identical
# key sequence and the collision pattern is identical too.
#
# The table is the built-in dict.

SEED = 88172645463325252
COUNT = 10000
KEY_SPACE = 1000000
MASK = (1 << 64) - 1


def xs_next(state):
    state ^= (state << 13) & MASK
    state ^= state >> 7
    state ^= (state << 17) & MASK
    return state


table = {}
state = SEED
for _ in range(COUNT):
    state = xs_next(state)
    key = state % KEY_SPACE
    table[key] = key

found = 0
total = 0
state = SEED
for _ in range(COUNT):
    state = xs_next(state)
    key = state % KEY_SPACE
    if key in table:
        found += 1
        total += key

print(len(table))
print(found)
print(total)
