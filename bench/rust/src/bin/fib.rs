// Naive recursive Fibonacci: call overhead and recursion, nothing else.

fn fib(n: i64) -> i64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

fn main() {
    println!("fib(32) = {}", fib(32));
}
