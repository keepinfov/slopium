// Naive recursive Fibonacci: call overhead and recursion, nothing else.

public class fib {
    static final int DEPTH = 32;

    static long fib(long n) {
        return n < 2 ? n : fib(n - 1) + fib(n - 2);
    }

    public static void main(String[] args) {
        System.out.println("fib(" + DEPTH + ") = " + fib(DEPTH));
    }
}
