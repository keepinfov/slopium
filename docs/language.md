# Slopium language v0.2

Slopium source consists of S-expressions. `;` starts a line comment. Integer,
floating-point, boolean (`true`, `false`), and escaped string literals are
supported.

## Files, modules, and imports

Every `.slp` file contains declarations. Its module name is derived from its
path below the package source root: `src/geometry/vector.slp` is
`geometry:vector`.

```lisp
; src/geometry.slp
(export Point distance (internal:Thing :as Thing))

(struct Point ((x i64) (y i64)))
(fn distance ((point Point)) -> i64
  (+ (. point x) (. point y)))
```

```lisp
; src/main.slp
(take std:io println-i64)
(take geometry Point (distance :as length))

(fn main () -> i32
  (let point (Point :x 20 :y 22))
  (println-i64 (length point))
  0)
```

`export` is the complete public API. `take` introduces explicit file-wide
aliases; there are no wildcard imports. A qualified name such as
`geometry:distance` still obeys privacy. `::` is not a valid separator.
All module dependency cycles are rejected.

## Declarations and generics

```lisp
(fn add ((a i64) (b i64)) -> i64
  (+ a b))

(fn identity (T) ((value T)) -> T value)
(struct Box (T) ((value T)))
(enum Option (T) None (Some ((value T))))

(test "addition"
  (= (add 20 22) 42))
```

Generic applications use S-expressions, for example `(Box String)` and
`(Result i64 Error)`. Type arguments are inferred at calls and constructors.
The compiler monomorphizes only reachable concrete instances. Traits and
bounds are intentionally not part of v0.2.

Scalar types are `unit`, `bool`, `i32`, `i64`, and `f64`. Other built-in types
are `String`, `(List T)`, `(Array T N)`, `(Slice T)`, `(& T)`, and `(&mut T)`.
Numeric conversions are never implicit.

## Ownership and borrows

```lisp
(let message "hello")
(let copy (clone message))
(println (& message))
```

Owned values move by default. `(& value)` and `(&mut value)` create shared and
exclusive borrows. A borrow ends after its last use where the control-flow
analysis can prove that it is dead; references still cannot escape a function
or be stored in aggregate fields or collection elements. Borrowed slices
cannot be returned either. `clone` recursively copies strings, lists, arrays,
structs, and enums. Generated drop glue recursively destroys them.

## Control flow

```lisp
(let mut counter 0)
(while (< counter 10)
  (set counter (+ counter 1))
  (if (= counter 5) (continue) ())
  (if (= counter 8) (break) ()))

(loop
  (set counter (+ counter 1))
  (if (= counter 10) (break) ()))
```

`if` has a value and both branches must have the same type. `do` evaluates a
sequence and returns its final expression. `while` and `loop` return `unit`;
`break` and `continue` do not take values.

## Structs, enums, and patterns

```lisp
(struct Point ((x i64) (label String)))
(enum Message
  Empty
  (Pointed ((point Point))))

(match (Message:Pointed (Point :x 42 :label "answer"))
  ((Message:Pointed (Point :x x :label label))
    (do (println (& label)) x))
  ((Message:Empty) 0))
```

Patterns can nest enum and named struct patterns. A bare name binds and moves
the matched value; `_` discards it. Boolean and enum matches must be
exhaustive or contain an irrefutable arm. Integer matches require a final
irrefutable arm.

## Collections

`List<T>` owns its elements, including non-`Copy` values.

```lisp
(let mut values (list "one" "two"))
(do (push (&mut values) "three"))
(let first (get-ref (& values) 0))
(println first)
(let removed (remove (&mut values) 1))
```

`get` copies only `Copy` elements. Use `get-ref` to borrow an element or
`remove` to move one out. With the standard `Option` language item,
`(pop (&mut values))` returns `Option<T>` and never panics for an empty list.
Out-of-range `get`, `get-ref`, `remove`, and `slice` remain deterministic
runtime errors with exit status 101.

```lisp
(let fixed (array "zero" "one" "two"))
(let view (slice (& fixed) 1 3))
(println-i64 (len (& view)))
```

`array` creates an owned fixed-length `Array<T, N>`. `slice` creates a
non-owning range descriptor tied to the lifetime of a borrowed list or array.
`len` also accepts `(& String)`, where it is the length in bytes.

## Standard `Option`, `Result`, and `try`

The bundled `std` dependency exports generic `Option` and `Result`. A project
enables it in `Slopium.toml`:

```toml
[dependencies]
std = { toolchain = true }
```

```lisp
(take std:prelude Result)

(fn forward () -> (Result i64 String)
  (let value (try (produce)))
  (Result:Ok value))
```

`try` is configured through replaceable language items, so a path dependency
can supply a compatible standard library instead of the bundled one.

## The library

Input, output, strings and files are not part of the language. They are
ordinary modules of the bundled library, written in Slopium over `extern`
declarations, and a program that uses one says so.

The library is two packages. `core` is what a program with no C library under
it can have — `option`, `result` and `string`. `std` is `core` plus what needs
an operating system — `io`, `process` and `fs` — and it re-exports `core`
through `std:prelude` and `std:string`, so a package that depends on `std`
alone reaches everything by that name.

```lisp
(take std:io println println-i64 read-i64)
(take std:string from-i64 split to-i64 trim)
(take std:process env)
(take std:prelude Option)

(let name "FLAG")
(match (env (& name))
  ((Option:Some flag) (println (& flag)))
  ((Option:None) ()))
```

**Nothing in the library aborts** (`D-087`). A line past the end of input, a
byte that is not a digit, an argument that is not there, a variable that is not
set: each is `None`, and a file operation that fails is an `Err` carrying an
`errno`. The runtime errors that remain — an index out of bounds, a division by
zero, an overflow — are the language's own, and they still print a normalized
message and exit with status 101.

`std:string` is bytes: `byte-at`, `substring`, `concat`, `from-bytes`,
`equals`, `starts-with`, `find`, `contains`, `trim`, `split` on a separator
byte, and `from-i64` and `to-i64` between a number and its text. `to-i64`
returns `(Option i64)` and refuses anything that is not an optional `-`
followed by digits, including a number too large to hold.

`std:io` has `print` and `println` over `(& String)`, `print-i64`,
`println-i64`, `print-bool` and `println-bool`, `read-line` returning
`(Option String)` without LF/CRLF, and `read-i64` returning `(Option i64)`.
There are no traits yet, so one name cannot print every printable type
(`D-078`); the widths are separate functions, and their bodies are Slopium over
`from-i64`. There is no `println-i32`, because there is no widening conversion
for an `i32` to reach `from-i64` through (`D-086`).

`std:fs` reads and writes whole files: `read`, `write`, `exists` and `delete`,
each taking a path, returning `(Result T Error)` where `Error` carries an
`errno`. There is no `open` and no `close` — a file descriptor in a Slopium
value would have nothing to close it, because a struct wrapping an `i64` owns
nothing and no drop glue runs for it (`D-084`).

`std:process` has `args-len`, `arg` and `args`, `env` returning
`(Option String)`, and `exit`.

A lone file has no manifest to declare a dependency in, so `slopic file.slp`
compiles it against `std` and `--no-std` opts out; `--freestanding` gets `core`
instead. A package says what it depends on:

```toml
[dependencies]
std = { toolchain = true }
```

## Calling C

An `extern` declaration names a C function and gives it a Slopium signature.
The string is the symbol the linker is asked for; the name after it is the one
Slopium calls, and it is an ordinary module-level item — private by default,
`export`able, `take`able, and canonicalized to `module:name`.

```lisp
(extern "strlen" (c-strlen (text (& String))) -> i64)
(extern "hal_scale" (hal-scale (value f64) (factor f64)) -> f64)

(fn main () -> i32
  (let text "borrowed")
  (println-i64 (c-strlen (& text)))
  0)
```

The type vocabulary is closed. A parameter is `i32`, `i64`, `f64`, `bool`,
`(& String)`, or `(& (Slice T))`. A return is `unit`, one of those scalars, or
an owned `String`. Anything else is refused at the declaration, and an `extern`
cannot be generic: there is nothing a type parameter could stand for.

An `extern` borrows and never moves. A `(& String)` arrives as a
NUL-terminated `const char *`; a `(& (Slice T))` arrives as two arguments, the
element pointer and then the element count. The borrow ends when the call
returns, so C must copy anything it intends to keep. To return a `String`, C
allocates one with the runtime's `sl_rt_string_new(const char *, uint64_t)`;
the caller owns it and drops it as it would any other.

Two things the vocabulary does not say for you. C's `size_t`, `unsigned` and
`long` have no Slopium spelling: they are declared as `i64` (or `i32`), and a
value that does not fit is your mistake to avoid, not one the compiler can
catch. And a variadic C function may not be declared at all — the System V ABI
wants the vector-register count in `al` for one, and nothing here sets it.

The C itself is the package's, listed in `Slopium.toml`:

```toml
[package]
c-sources = ["c/hal.c"]
```

Those paths are relative to the package root and may not leave it. They are
compiled with the same `cc` the link uses, ship in the package archive, and
their contents are part of the build cache key, so editing one rebuilds.
