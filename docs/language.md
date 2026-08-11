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
The compiler monomorphizes only reachable concrete instances. A generic
function may take, match, build and return a generic type, so
`(fn map (T U) ((value (Option T)) (f (Fn (T) U))) -> (Option U) ...)` is an
ordinary declaration; inside such a body the type is still an application and
not an instance, which is why `try` refuses one (`D-095`). Inference is
positional and left to right: an argument is typed against the parameter it is
passed to, so a value that says nothing about a parameter on its own — an empty
`(list)`, an `(Option:None)` — needs an earlier argument or the expected type
to have settled it. The language has
no traits and no bounds, and none are planned for 1.0 (`D-088`). A bound can be
added later to a parameter that is unconstrained today without invalidating a
program that already satisfies it, which is why refusing them now costs a
future version nothing.

Scalar types are `unit`, `bool`, `i32`, `i64`, and `f64`. Other built-in types
are `String`, `(List T)`, `(Array T N)`, `(Slice T)`, `(& T)`, `(&mut T)`, and
`(Fn (T ...) R)`.
Numeric conversions are never implicit. `(as i64 value)` is the one that exists
(`D-090`): it widens an `i32`, and every other pair — narrowing, truncating,
anything touching `f64` — is refused by name. The form takes a target type
rather than a value, so what it converts to is read the way a type is read and
not the way a variable is. Turning an `f64` into an integer is not in the
vocabulary at all; turning one into text is `from-f64`, in the library.

`+`, `-`, `*`, `/`, `<`, and `>` take two operands of one numeric type. `=`
takes two `bool`, `i32`, `i64`, or `f64` operands and nothing else (`D-089`):
comparing two `String`s, two structs, two enums, two borrows, or two values of
an unconstrained type parameter is an error. Text is compared with
`core:string:equals`, which `std:string` re-exports. Without traits there is no
way to give `=` a meaning for a type the compiler did not define, and comparing
such values by the machine word that holds them would answer about identity
while looking like it answered about contents.

## Function types

```lisp
(fn double ((value i64)) -> i64
  (* value 2))

(fn apply ((f (Fn (i64) i64)) (value i64)) -> i64
  (f value))

(fn main () -> i32
  (println-i64 (apply double 21))
  0)
```

A function type is written `(Fn (parameter ...) result)`. The parameters are
grouped in their own list, so every arity has exactly one spelling: `(Fn () i64)`
takes nothing, `(Fn (i64) i64)` takes one, and nothing has to be counted from the
right to find where the result begins.

A top-level `fn` named where a value is expected *is* that function, and a local
of `Fn` type in head position is called through. Both are ordinary lookups with
one rule between them: the function namespace is consulted first, so a call
`(f v)` means the `fn` named `f` whenever there is one, and a local named `f` of
`Fn` type beside a `fn f` is an error rather than a silent winner. A local of any
other type may share a name with a `fn` as it always could.

A function value is one machine word — the address of a top-level function. It
is `Copy`, so passing it twice is not a move and `clone` refuses it the way it
refuses a scalar; it can be returned, and it can be a struct or enum field.

An `extern` is not a value. Its arguments may cross the C boundary as more than
one machine word — a borrowed `Slice` goes as a pointer and a length — so a `Fn`
type cannot describe the call it makes. Wrap it in a `fn` and take that instead.

A generic function used as a value needs its instance chosen where the value is
taken, because a value is the address of one monomorphized body. Where the
expected type says which instance that is, it is taken; where it does not, it is
refused with `SL0452` rather than guessed at.

There are no closures and no `lambda` yet: a function value captures nothing.

## Ownership and borrows

```lisp
(let message "hello")
(let copy (clone message))
(println (& message))

(fn owned ((text (& String))) -> String
  (clone text))
```

Owned values move by default. `(& value)` and `(&mut value)` create shared and
exclusive borrows. A borrow ends after its last use where the control-flow
analysis can prove that it is dead; references still cannot escape a function
or be stored in aggregate fields or collection elements. Borrowed slices
cannot be returned either. `clone` recursively copies strings, lists, arrays,
structs, and enums. Generated drop glue recursively destroys them.

`clone` crosses a borrow (`D-091`): `(clone text)` on a `(& String)` is a
`String`, which is how a borrowed value is copied into an owned one. It refuses
a `&mut`, and it refuses a scalar — a `bool`, an `i32`, an `i64` or an `f64` is
copied by being used, so `(clone 42)` is an error rather than a call that does
nothing.

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

An empty `(list)` or `(array)` is legal wherever the expected type says what it
holds — a return position, an argument, an arm of an `if` or a `match` — and an
error only where nothing does (`D-096`). It is the same rule `(Option:None)`
follows.

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
it can have — `option`, `result`, `list`, `string` and `float`. `std` is `core`
plus what needs an operating system — `io`, `process` and `fs` — and it
re-exports `core` through `std:prelude`, `std:option`, `std:result`,
`std:list`, `std:string` and `std:float`, so a package that depends on `std`
alone reaches everything by
that name. The combinators live in modules of their own rather than in
`prelude` because `option` and `result` both call theirs `map`.

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

`std:option` and `std:result` are the combinators. `Option` has `map`,
`and-then`, `unwrap-or` and `or-else`; `Result` has those plus `map-err` and
`ok`, which forgets why it failed and gives back an `Option`. These are what
refusing traits costs, and having them is what makes the refusal honest
(`D-088`). Each takes its value **by ownership**, because `match` does not work
through a borrow; that is also why there is no `is-some`, which would have to
consume what it only wanted to look at.

`std:list` is `map`, `filter`, `fold`, `find` and `sort-by`. Two shapes of
function appear here, and the difference is not stylistic. A function that
consumes each element takes the element: `map` is `(Fn (T) U)` and `fold` is
`(Fn (A T) A)`. A function that only looks at one takes **the borrowed list and
an index**: `filter` and `find` are `(Fn ((& (List T)) i64) bool)` and
`sort-by` is `(Fn ((& (List T)) i64 i64) bool)`, answering whether the first
index belongs ahead of the second. Looking at an element without consuming it
means borrowing it, and there is no way to read a `(& T)` — the language has no
dereference, and `get` copies only a `Copy` element, which an unconstrained `T`
is not. A borrowed list and an index is the one form that works for every `T`,
because the caller writing the predicate knows what `T` is. `find` answers with
an index, like `core:string:find`, because answering with the element would
have to move it out of a list the caller still owns.

```lisp
(take std:list filter sort-by)

(fn odd ((items (& (List i64))) (index i64)) -> bool
  (let value (get items index))
  (= 1 (- value (* (/ value 2) 2))))

(fn ascending ((items (& (List i64))) (left i64) (right i64)) -> bool
  (< (get items left) (get items right)))

(let kept (sort-by (filter (list 5 2 3 9) odd) ascending))
```

`sort-by` is stable, and both it and every consuming function here are
quadratic: sorting in place would need a way to overwrite one element, and the
language has none.

`std:string` is bytes: `byte-at`, `substring`, `concat`, `from-bytes`,
`equals`, `starts-with`, `find`, `contains`, `trim`, `split` on a separator
byte, and `from-i64` and `to-i64` between a number and its text. `to-i64`
returns `(Option i64)` and refuses anything that is not an optional `-`
followed by digits, including a number too large to hold.

`std:float` is the float, kept apart from `std:string` and `std:io` rather
than split between them: `from-f64`, `to-f64`, `print-f64`, `println-f64` and
`read-f64`. A whole module is one section of code, so taking one brings
everything it calls, and putting these beside `from-i64` and `println-i64` made
the smallest program that prints a word grow from 11,692 bytes of code to
26,484. A program pays for a float when it mentions one.

**A printed `f64` is plain decimal** — an optional `-`, digits, a
`.`, and digits, with at least one digit on each side of the point — rounded to
seventeen significant digits, ties to even, with trailing fractional zeros
removed and one always kept, so `1.0` prints as `1.0` and `0.1` prints as
`0.10000000000000001`. `nan`, `inf` and `-inf` name the three values that have
no digits, and `-0.0` prints its sign.

There is no exponent form and there will not be one while the language has no
exponent literal (`D-098`): `1.5e10` is not source, so printing an exponent
would produce text the compiler could not read back. Plain decimal costs
nothing but length, and only at the extremes — the largest `f64` is 309 digits
and the smallest subnormal is 342 characters. Seventeen digits is the width at
which `(to-f64 (from-f64 v))` is `v` for every `v`, and `from-f64` is the only
way to observe an `f64`, so it does not round away bits that nothing else could
recover.

`to-f64` accepts everything `from-f64` writes, plus a whole number with no
point, and answers `None` for anything else. It is correctly rounded, ties to
even, over the whole range: a value too large for the type reads as an infinity
and one too small as a zero, because both are values of the type. That is
unlike `to-i64`, which answers `None` on overflow — an `i64` has nothing to
overflow to.

None of this is C. `core:float` is Slopium over two runtime primitives that
read and write the bit pattern of a double and do nothing else, so a program
with no C library can print a number it computed (`D-097`). The digits are
exact rather than approximate: a double is `significand * 2^exponent`, and that
product is a finite decimal, reached by multiplying an integer by two or by
five and never by scaling the float itself.

`std:io` has `print` and `println` over `(& String)`, `print-i64`,
`println-i64`, `print-bool` and `println-bool`, `read-line` returning
`(Option String)` without LF/CRLF, and `read-i64` returning `(Option i64)`.
There are no traits and none are planned (`D-088`), so one name cannot print
every printable type (`D-078`); the widths are separate functions, and their
bodies are Slopium over `from-i64`. There is no `println-i32` and there will
not be one: an `i32` reaches `println-i64` as `(println-i64 (as i64 value))`,
which is the debt `D-086` named and `D-090` paid. `println-f64` is in
`std:float` for the size reason above, not because it is a different kind of
thing.

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
