# Naive matrix multiply, i-j-k order, flat row-major lists. The second
# floating-point kernel: it adds memory access patterns to the arithmetic.
# The order of the three loops and the shape of every expression are the
# same in every language, down to the association, because the results are
# compared as floats and last-ulp differences are real differences here.
#
# The fill loops run on float counters stepped by 1.0, exactly as in the
# Slopium entry, which has no integer-to-float conversion. The checksum is
# the sum of every entry of the product, printed through the language's own
# float formatter and compared with a tolerance.

SIZE = 160

n = 160.0
a = []
b = []
rf = 0.0
while rf < n:
    cf = 0.0
    while cf < n:
        a.append(0.001 * (3.0 * rf + 7.0 * cf + 1.0))
        b.append(0.002 * (5.0 * cf + 2.0 * rf + 0.25))
        cf += 1.0
    rf += 1.0

c = []
for i in range(SIZE):
    for j in range(SIZE):
        acc = 0.0
        for k in range(SIZE):
            acc += a[i * SIZE + k] * b[k * SIZE + j]
        c.append(acc)

total = 0.0
for entry in c:
    total += entry

print(f"sum = {total}")
