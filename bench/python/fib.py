# Naive recursive Fibonacci: call overhead and recursion, nothing else.

DEPTH = 32


def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)


print(f"fib({DEPTH}) = {fib(DEPTH)}")
