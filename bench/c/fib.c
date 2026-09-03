/* Naive recursive Fibonacci: call overhead and recursion, nothing else. */

#include <stdio.h>

#define DEPTH 32

static long long fib(long long n)
{
    return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

int main(void)
{
    printf("fib(%d) = %lld\n", DEPTH, fib(DEPTH));
    return 0;
}
