// Naive matrix multiply, i-j-k order, flat row-major vectors. The second
// floating-point kernel: it adds memory access patterns to the arithmetic.
// The order of the three loops and the shape of every expression are the
// same in every language, down to the association, because the results are
// compared as floats and last-ulp differences are real differences here.
//
// The fill loops run on f64 counters stepped by 1.0, exactly as in the
// Slopium entry, which has no integer-to-float conversion. The checksum is
// the sum of every entry of the product, printed through the language's own
// float formatter and compared with a tolerance.

const SIZE: usize = 160;

fn main() {
    let n = 160.0;
    let mut a: Vec<f64> = Vec::with_capacity(SIZE * SIZE);
    let mut b: Vec<f64> = Vec::with_capacity(SIZE * SIZE);
    let mut rf = 0.0;
    while rf < n {
        let mut cf = 0.0;
        while cf < n {
            a.push(0.001 * (3.0 * rf + 7.0 * cf + 1.0));
            b.push(0.002 * (5.0 * cf + 2.0 * rf + 0.25));
            cf += 1.0;
        }
        rf += 1.0;
    }
    let mut c: Vec<f64> = Vec::with_capacity(SIZE * SIZE);
    for i in 0..SIZE {
        for j in 0..SIZE {
            let mut acc = 0.0;
            for k in 0..SIZE {
                acc += a[i * SIZE + k] * b[k * SIZE + j];
            }
            c.push(acc);
        }
    }
    let mut sum = 0.0;
    for entry in &c {
        sum += entry;
    }
    println!("sum = {}", sum);
}
