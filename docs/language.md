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
(take geometry Point (distance :as length))

(fn main () -> i32
  (let point (Point :x 20 :y 22))
  (println (length point))
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
(println (len (& view)))
```

`array` creates an owned fixed-length `Array<T, N>`. `slice` creates a
non-owning range descriptor tied to the lifetime of a borrowed list or array.

## Standard `Option`, `Result`, and `try`

The bundled `std` dependency exports generic `Option` and `Result`. A project
enables it in `Slopium.toml`:

```toml
[dependencies]
std = { toolchain = true }
```

```lisp
(take std:result Result)

(fn forward () -> (Result i64 String)
  (let value (try (produce)))
  (Result:Ok value))
```

`try` is configured through replaceable language items, so a path dependency
can supply a compatible standard library instead of the bundled one.

## Console, environment, and process arguments

`read-i64` reads one signed decimal integer. `read-line` returns an owned line
without LF/CRLF. `parse-i64` validates a borrowed string, and `env` copies an
environment variable into an owned `String`.

```lisp
(let number (read-i64))
(let name "FLAG")
(let flag (env (& name)))
(println number)
(println (& flag))
```

`args-len` excludes the executable name; `arg` returns an owned argument copy.
Unrecoverable runtime errors print a normalized message and exit with status
101.

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
  (println (c-strlen (& text)))
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
