// Naive matrix multiply, i-j-k order, flat row-major arrays. The second
// floating-point kernel: it adds memory access patterns to the arithmetic.
// The order of the three loops and the shape of every expression are the
// same in every language, down to the association, because the results are
// compared as floats and last-ulp differences are real differences here.
//
// The fill loops run on double counters stepped by 1.0, exactly as in the
// Slopium entry, which has no integer-to-float conversion. The checksum is
// the sum of every entry of the product, printed through the language's own
// float formatter and compared with a tolerance.

public class matmul {
    static final int SIZE = 160;

    public static void main(String[] args) {
        double n = 160.0;
        double[] a = new double[SIZE * SIZE];
        double[] b = new double[SIZE * SIZE];
        for (double rf = 0.0; rf < n; rf += 1.0) {
            for (double cf = 0.0; cf < n; cf += 1.0) {
                int row = (int) rf, col = (int) cf;
                a[row * SIZE + col] = 0.001 * (3.0 * rf + 7.0 * cf + 1.0);
                b[row * SIZE + col] = 0.002 * (5.0 * cf + 2.0 * rf + 0.25);
            }
        }
        double[] c = new double[SIZE * SIZE];
        for (int i = 0; i < SIZE; i++)
            for (int j = 0; j < SIZE; j++) {
                double acc = 0.0;
                for (int k = 0; k < SIZE; k++)
                    acc += a[i * SIZE + k] * b[k * SIZE + j];
                c[i * SIZE + j] = acc;
            }
        double sum = 0.0;
        for (int i = 0; i < SIZE * SIZE; i++)
            sum += c[i];
        System.out.println("sum = " + sum);
    }
}
