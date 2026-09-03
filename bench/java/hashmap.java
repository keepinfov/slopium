// Hash map workload: 10,000 inserts of key->key, then the same 10,000 keys
// looked up again. Keys come from a three-shift xorshift64 over a fixed
// seed, reduced modulo one million, so every language walks the identical
// key sequence and the collision pattern is identical too.
//
// The table is java.util.HashMap with its default hasher; the program's
// output does not depend on the hasher, only its speed does.

import java.util.HashMap;

public class hashmap {
    static final long SEED = 88172645463325252L;
    static final int COUNT = 10000;
    static final long KEY_SPACE = 1000000;

    static long xsNext(long state) {
        state ^= state << 13;
        state ^= state >>> 7;
        state ^= state << 17;
        return state;
    }

    public static void main(String[] args) {
        HashMap<Long, Long> table = new HashMap<>();
        long state = SEED;
        for (int i = 0; i < COUNT; i++) {
            state = xsNext(state);
            long key = Long.remainderUnsigned(state, KEY_SPACE);
            table.put(key, key);
        }
        long found = 0;
        long sum = 0;
        state = SEED;
        for (int i = 0; i < COUNT; i++) {
            state = xsNext(state);
            long key = Long.remainderUnsigned(state, KEY_SPACE);
            if (table.containsKey(key)) {
                found++;
                sum += key;
            }
        }
        System.out.println(table.size());
        System.out.println(found);
        System.out.println(sum);
    }
}
