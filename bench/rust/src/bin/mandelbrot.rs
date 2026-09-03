// Mandelbrot, ASCII PBM. The floating-point kernel: escape-time iteration
// over the classic window, fifty iterations, one character per pixel.
//
// The pixel counters are f64 stepped by 1.0, exactly as in the Slopium
// entry, so every language performs the same operations in the same order
// and the image is bit-identical. Output is P1: 1 for a point inside the
// set, 0 for one outside.

const SIZE: f64 = 200.0;
const ITERATIONS: i32 = 50;

fn main() {
    print!("P1\n200 200\n");
    let step = 2.0 / SIZE;
    let mut py = 0.0;
    while py < SIZE {
        let ci = step * py - 1.0;
        let mut row = String::with_capacity(200);
        let mut px = 0.0;
        while px < SIZE {
            let cr = step * px - 1.5;
            let mut zr = 0.0;
            let mut zi = 0.0;
            let mut i = 0;
            while i < ITERATIONS && zr * zr + zi * zi <= 4.0 {
                let t = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = t;
                i += 1;
            }
            row.push(if i == ITERATIONS { '1' } else { '0' });
            px += 1.0;
        }
        println!("{}", row);
        py += 1.0;
    }
}
