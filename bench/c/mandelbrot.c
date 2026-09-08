/* Mandelbrot, ASCII PBM. The floating-point kernel: escape-time iteration
 * over the classic window, fifty iterations, one character per pixel.
 *
 * The pixel counters are doubles stepped by 1.0, exactly as in the Slopium
 * entry, so every language performs the same operations in the same order and
 * the image is bit-identical. Output is P1: 1 for a point inside the set,
 * 0 for one outside. */

#include <stdio.h>

#define SIZE 200.0
#define ITERATIONS 50

int main(void)
{
    fputs("P1\n200 200\n", stdout);
    double step = 2.0 / SIZE;
    for (double py = 0.0; py < SIZE; py += 1.0)
    {
        double ci = step * py - 1.0;
        for (double px = 0.0; px < SIZE; px += 1.0)
        {
            double cr = step * px - 1.5;
            double zr = 0.0;
            double zi = 0.0;
            int i = 0;
            while (i < ITERATIONS && zr * zr + zi * zi <= 4.0)
            {
                double t = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = t;
                i++;
            }
            putchar(i == ITERATIONS ? '1' : '0');
        }
        putchar('\n');
    }
    return 0;
}
