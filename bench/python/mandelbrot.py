# Mandelbrot, ASCII PBM. The floating-point kernel: escape-time iteration
# over the classic window, fifty iterations, one character per pixel.
#
# The pixel counters are floats stepped by 1.0, exactly as in the Slopium
# entry, so every language performs the same operations in the same order and
# the image is bit-identical. Output is P1: 1 for a point inside the set,
# 0 for one outside.

SIZE = 200.0
ITERATIONS = 50

print("P1\n200 200\n", end="")
step = 2.0 / SIZE
py = 0.0
while py < SIZE:
    ci = step * py - 1.0
    row = []
    px = 0.0
    while px < SIZE:
        cr = step * px - 1.5
        zr = 0.0
        zi = 0.0
        i = 0
        while i < ITERATIONS and zr * zr + zi * zi <= 4.0:
            t = zr * zr - zi * zi + cr
            zi = 2.0 * zr * zi + ci
            zr = t
            i += 1
        row.append("1" if i == ITERATIONS else "0")
        px += 1.0
    print("".join(row))
    py += 1.0
