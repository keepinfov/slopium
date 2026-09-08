// Hash map workload: 10,000 inserts of key->key, then the same 10,000 keys
// looked up again. Keys come from a three-shift xorshift64 over a fixed
// seed, reduced modulo one million, so every language walks the identical
// key sequence and the collision pattern is identical too.
//
// The table is std::collections::HashMap with its default hasher; the
// program's output does not depend on the hasher, only its speed does.

use std::collections::HashMap;

const SEED: u64 = 88172645463325252;
const COUNT: i64 = 10000;
const KEY_SPACE: u64 = 1000000;

fn xs_next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn main() {
    let mut state = SEED;
    let mut table: HashMap<i64, i64> = HashMap::new();
    for _ in 0..COUNT {
        let key = (xs_next(&mut state) % KEY_SPACE) as i64;
        table.insert(key, key);
    }
    let mut found = 0;
    let mut sum = 0;
    state = SEED;
    for _ in 0..COUNT {
        let key = (xs_next(&mut state) % KEY_SPACE) as i64;
        if table.contains_key(&key) {
            found += 1;
            sum += key;
        }
    }
    println!("{}", table.len());
    println!("{}", found);
    println!("{}", sum);
}
