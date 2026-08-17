# Slopium language v0.2

Slopium source consists of S-expressions. `;` starts a line comment. Integer,
floating-point, boolean (`true`, `false`), and escaped string literals are
supported.

An integer literal is decimal, hexadecimal (`0xB8000`) or binary (`0b1010`),
and `_` may appear between its digits: `1_000_000`, `0xdead_beef`. **A
hexadecimal or binary literal is a bit pattern and a decimal one is a number**,
so `0xFFFF_FFFF_FFFF_FFFF` is `-1` and `0x8000_0000_0000_0000` is the smallest
`i64`, while the same values written in decimal are out of range and refused.
A mask is not a magnitude, and a driver that had to spell one as a negative
decimal would be a driver nobody could review.

A string literal is bytes, not characters. Besides `\n`, `\r`, `\t`, `\"` and
`\\` it takes `\0` and `\xNN`, and **`\xNN` is exactly one byte** for every
`NN` from `00` to `ff`. A `String` is a length and a buffer, so a literal may
hold a NUL or any other byte and `len` counts bytes.

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

Scalar types are `unit`, `bool`, `f64`, and the eight integers `i8`, `i16`,
`i32`, `i64`, `u8`, `u16`, `u32` and `u64` (`D-107`). Other built-in types are
`String`, `(List T)`, `(Array T N)`, `(Slice T)`, `(& T)`, `(&mut T)`, and
`(Fn (T ...) R)`. Every integer is one machine word wide whatever its type
says, so a `(List u8)` is a word per element; the type decides what the word
means, not how much room it takes.

Numeric conversions are never implicit (`D-090`). `(as T value)` converts
between the integer types and nothing else: **the source's signedness extends
and the target's width truncates.** So `(as u64 (as i8 -1))` is every bit set,
`(as u8 (as i8 -1))` is `255`, and `(as i8 0xFF)` is `-1`. Every pair of
integer types is legal in both directions and none of it traps — a conversion
describes a pattern of bits, and `D-031`'s overflow checks are about
arithmetic. Anything touching `f64` or `bool` is refused by name. The form
takes a target type rather than a value, so what it converts to is read the way
a type is read and not the way a variable is. Turning an `f64` into an integer
is not in the vocabulary at all; turning one into text is `from-f64`, in the
library.

An integer literal takes its type from what is expected of it and is `i64`
otherwise, and it must fit there: `255` is a `u8` and not an `i8`. A
hexadecimal or binary literal is a bit pattern rather than a number (`D-112`),
which it stays at every width — `0xFF` is `255` as a `u8` and `-1` as an `i8`,
while decimal `-1` is an `i8` and not a `u8` at all.

`+`, `-`, `*`, `/`, `<`, `>`, `<=`, and `>=` take two operands of one numeric
type. `%` is the remainder and takes two integers; it truncates so that
`(= a (+ (* (/ a b) b) (% a b)))` holds for every pair, matching `/`, and it
traps on a zero divisor exactly as `/` does. `=` and `!=` take two `bool`,
integer, or `f64` operands and nothing else (`D-089`): comparing two
`String`s, two structs, two enums, two borrows, or two values of an
unconstrained type parameter is an error. Text is compared with
`core:string:equals`, which `std:string` re-exports. Without traits there is no
way to give `=` a meaning for a type the compiler did not define, and comparing
such values by the machine word that holds them would answer about identity
while looking like it answered about contents.

`(- x)` with one operand is negation, and it traps on the smallest integer for
the reason `(- 0 x)` does. It is refused outright on an unsigned type, where
the only value it could answer for is zero.

Arithmetic on the two operands' shared type, and every type keeps its own
range: a `u8` addition overflows above `255` and an `i8` one above `127`, and
both trap rather than wrapping (`D-031`). Mixing types is an error — a `u8`
and an `i64` do not add — because `D-090` says a conversion is written down.

`bit-and`, `bit-or`, `bit-xor` and `bit-not` are the bitwise operations and
`shl` and `shr` the shifts, all on integers. They are spelled out because `&`
is a borrow and a language where `(& a b)` is a bitwise and while `(& a)` is a
borrow has a trap in it. `shr` is arithmetic on a signed type and logical on an
unsigned one, which is what the two words mean and needs no second operator.
**A shift by a negative amount, or by the width of the type or more, traps** — the two
architectures disagree about what such a shift would otherwise mean, and
neither answer is one a program asked for. **A shift does not trap when bits
leave the top**: `(shl 1 63)` is the smallest `i64` and that is the answer, not
an overflow, because a shift describes a pattern of bits rather than a
magnitude. Both rules read the *type's* width, so a `u8` shifted by 8 traps and
one shifted by 7 is `128`.

`and` and `or` are forms rather than calls, because they stop at the operand
that answers: `(and (holds table key) (trust (lookup table key)))` does not
look the key up when the table does not hold it. Each takes two operands or
more and every one must be a `bool`. `not` is an ordinary operator over one.

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

A function value is one machine word and it is **owned**, like a `String`: it is
dropped when it goes out of scope, `clone` copies it, and handing it to
something else is a move. A function that only wants to *call* one takes it by
value or by borrow — calling never consumes — but a function that wants to pass
it on twice must take a `(& (Fn ...))`, which is callable like the thing it
borrows. It can be returned and it can be a struct or enum field.

An `extern` is not a value. Its arguments may cross the C boundary as more than
one machine word — a borrowed `Slice` goes as a pointer and a length — so a `Fn`
type cannot describe the call it makes. Wrap it in a `fn` and take that instead.

A generic function used as a value needs its instance chosen where the value is
taken, because a value is the address of one monomorphized body. Where the
expected type says which instance that is, it is taken; where it does not, it is
refused with `SL0452` rather than guessed at.

## Closures

```lisp
(let offset 3)
(let add (lambda (offset) ((x i64)) -> i64 (+ x offset)))
(println-i64 (add 39))
```

A `lambda` is a `fn` with the name dropped and one list changed: where a
declaration writes the type parameters it is parameterised over, a `lambda`
writes the names it closes over. Everything else — the parameter list, the `->`,
the result type, the body — is the same.

**A capture is a move, and it is written down.** `(lambda (offset) ...)` moves
`offset` into the closure, which owns it from then on; afterwards the outer
`offset` is gone, exactly as if it had been passed to a function. Nothing is
inferred: a name the body uses and the capture list does not name is an error
that says so. To keep a copy, `clone` it first.

Inside the body a capture keeps its own name and its own type, so the body reads
as it would have read in place. What it may not do is give a capture away: the
closure owns it and may be called again, so moving one out is refused. Borrow it
or clone it instead.

A closure and a plain `fn` value have the same type and are used
interchangeably, which is what lets `(filter items keep)` take either:

```lisp
(let limit 4)
(let kept (filter numbers (lambda (limit) ((n (& i64))) -> bool (> (clone n) limit))))
```

Because a closure owns its captures, it may outlive the function that built one:

```lisp
(fn greeter ((who String)) -> (Fn ((& String)) String)
  (lambda (who) ((mark (& String))) -> String
    (concat (& who) mark)))
```

**A capture may not be a borrow.** Because a closure can outlive the frame it
was written in, an environment holding a `(& T)` or a `Slice` is the same
mistake as returning one, and it is refused by the same rule. Capture what the
borrow points at, or `clone` it.

A `lambda` has no name, so it cannot call itself, and it takes no type
parameters of its own — one written inside a generic function is monomorphized
along with it.

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

`clone` crosses a borrow (`D-091`, `D-100`): `(clone text)` on a `(& String)` is
a `String`, and `(clone n)` on a `(& i64)` is an `i64`. This is how a borrowed
value is read, and it is the only way — the language has no dereference operator
and none is planned, because through a borrow this form already is one. It
refuses a `&mut`. It refuses an *owned* scalar: a `bool`, an `i32`, an `i64` or
an `f64` is copied by being used, so `(clone 42)` is an error rather than a call
that does nothing. Reading one out of a borrow is never nothing, which is why
the two cases differ.

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

A `match` also looks through a **shared borrow** of an enum or a struct
(`D-099`), and then it takes nothing apart: the value stays where it was, and
every name a pattern binds is a borrow of the field it names.

```lisp
(fn is-pointed ((message (& Message))) -> bool
  (match message
    ((Message:Pointed _) true)
    ((Message:Empty) false)))

(fn x-of ((point (& Point))) -> i64
  (match point
    ((Point :x x :label _) (clone x))))
```

`x` there is a `(& i64)`, not an `i64`, and `clone` is what reads it. The
binding is a borrow for **every** field type, including a `Copy` one: inside a
generic body, whether `T` is `Copy` is not known, and a binding's type has to
be. An exclusive borrow cannot be matched, and a borrowed scalar has nothing to
take apart — use `clone` and match the value.

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
Out-of-range `get`, `get-ref`, `remove`, `replace`, and `slice` remain
deterministic runtime errors with exit status 101.

```lisp
(let displaced (replace (&mut values) 0 "ONE"))
```

`replace` is the only write to an element there is, and it is a swap rather
than an assignment: the new value goes into the slot and **the old one is
returned**, owned, for the caller to keep or drop (`D-103`). It is what a
container written in Slopium is built out of — without it a list can only be
changed at its ends, and `core:map` would have to move every bucket to touch
one. `set` still assigns to a name and to nothing else: there is no assignment
to a field and none to an element.

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
it can have — `option`, `result`, `list`, `string`, `float`, `map` and `set`.
`std` is `core` plus what needs an operating system — `io`, `process` and
`fs` — and it re-exports `core` through `std:prelude`, `std:option`,
`std:result`, `std:list`, `std:string`, `std:float`, `std:map` and `std:set`,
so a package that depends on `std` alone reaches everything by
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

`std:option` and `std:result` are the combinators. `Option` has `is-some`,
`is-none`, `map`, `and-then`, `unwrap-or` and `or-else`; `Result` has `is-ok`,
`is-err`, those four, plus `map-err` and `ok`, which forgets why it failed and
gives back an `Option`. These are what refusing traits costs, and having them is
what makes the refusal honest (`D-088`). A combinator takes its value **by
ownership**, because it makes a new one out of it; a question takes a borrow and
leaves the value where it was.

`std:list` is `map`, `filter`, `fold`, `find` and `sort-by`. Two shapes of
function appear here, and the difference is ownership rather than style. A
function that consumes each element takes the element: `map` is `(Fn (T) U)` and
`fold` is `(Fn (A T) A)`. A function that only looks at one takes a borrow of
it: `filter` and `find` are `(Fn ((& T)) bool)` and `sort-by` is
`(Fn ((& T) (& T)) bool)`, answering whether the first element belongs ahead of
the second. `find` answers with an index, like `core:string:find`, because
answering with the element would have to move it out of a list the caller still
owns.

```lisp
(take std:list filter sort-by)

(fn odd ((item (& i64))) -> bool
  (let value (clone item))
  (= 1 (- value (* (/ value 2) 2))))

(fn ascending ((left (& i64)) (right (& i64))) -> bool
  (< (clone left) (clone right)))

(let kept (sort-by (filter (list 5 2 3 9) odd) ascending))
```

`sort-by` is stable, and both it and every consuming function here are
quadratic: each removal from the front moves the rest of the list. `replace`
now makes an in-place sort writable, and nothing here has been rewritten to use
it — the signatures do not change either way, so it is a later patch's work and
not an interface promise.

`std:string` is bytes: `byte-at`, `substring`, `concat`, `from-bytes`,
`equals`, `starts-with`, `find`, `contains`, `trim`, `split` on a separator
byte, `hash`, and `from-i64` and `to-i64` between a number and its text.
`to-i64` returns `(Option i64)` and refuses anything that is not an optional
`-` followed by digits, including a number too large to hold. `hash` is
`(h * 31 + byte)` kept under 2^31 - 1, which is a prime rather than a power of
two because arithmetic traps here: the usual mixing constants are written for a
language where overflow wraps.

`std:map` and `std:set` are a hash map and a hash set, and neither knows what a
key is. Both take the two functions that make a key a key:

```lisp
(take std:string hash equals)
(take std:map Map map-new map-insert map-lookup map-size)
(take std:option unwrap-or)

; An empty container takes its type from where it is written, and a `let`
; carries no type — so an empty map is written as the result of a function.
(fn empty-scores () -> (Map String i64)
  (map-new hash equals))

(let mut scores (empty-scores))
(set scores (map-insert scores "ann" 3))
(let key "ann")
(let held (unwrap-or (map-lookup (& scores) (& key)) 0))
```

`map-new` takes a `(Fn ((& K)) i64)` and a `(Fn ((& K) (& K)) bool)`; a key
type that does not have them yet gets them written for it, in Slopium, at the
call. **Everything that writes consumes the map and gives it back** —
`map-insert` and `map-delete` return the map — because the language assigns to
a name and to nothing else. Reading does not: `map-lookup`, `map-contains`,
`map-size` and `map-fold` take `(& (Map K V))`. `map-lookup` answers
`(Option V)` with the value cloned out, since a reference cannot leave the
function that made it.

`map-fold` is the only way to walk a map, and its accumulator comes from the
caller — `(map-fold m start step)` with `step` a `(Fn (A (& K) (& V)) A)`.
There is no `keys` and no iterator: an iterator is a lazy sequence, which is a
closure plus a protocol, and the protocol is a trait (`D-088`).

`std:set` is the same machine with the value left out: `set-of`, `set-add`,
`set-holds`, `set-discard`, `set-count` and `set-each`. A `Set` **is** a
`(Map T bool)`, written in Slopium over the map like anything else.

None of this needed traits, which is the whole point of it (`D-104`): `D-062`
made `Map` a non-goal on the grounds that a generic container over a comparable
key needs a bound, and it needs two function values instead.

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

## Raw pointers and volatile

A device register is a byte, a half or a word at a fixed address, and reaching
one is the only thing in the language the compiler cannot prove safe. So it is
spelled out: `(Ptr T)` is a raw pointer, and every operation on one is written
inside an `unsafe` block.

```lisp
(fn clear-screen ((vga (Ptr u16))) -> unit
  (unsafe
    (let mut cell 0)
    (while (< cell 2000)
      (volatile-write (ptr-offset vga cell) 0x0720)
      (set cell (+ cell 1)))))
```

The pointee must be a scalar — one of the eight integers, `bool`, or `f64`.
`(Ptr String)` and `(Ptr (List i64))` are refused at the type, which is what
keeps ownership out of the question entirely: nothing owned can be reached
through a pointer, so there is no aliasing rule to suspend.

A pointer is a scalar itself: one machine word, `Copy`, dropped by nobody. It
converts to and from any integer with `as`, in both directions, and to another
pointer:

| form | meaning |
| --- | --- |
| `(as (Ptr T) n)` | the address `n`, as a pointer to `T` |
| `(as u64 p)` | the address in `p` |
| `(as (Ptr U) p)` | the same address, read as a `U` |
| `(ptr-offset p n)` | `n` elements past `p`, scaled by `T`'s width |
| `(volatile-read p)` | the `T` at `p` |
| `(volatile-write p v)` | store `v` at `p` |

`ptr-offset` counts elements and not bytes, and its count is a `u64`, so the
arithmetic is unsigned throughout; it traps on overflow like any other. To go
backwards, compute the base you want.

Two things `unsafe` does **not** do.

It does not turn off the bounds and overflow checks. Indexing a list past its
end still panics inside an `unsafe` block, and so does an addition that leaves
its type. What the word buys is a pointer, not permission to skip a check.

And it does not travel. A `lambda` written inside an `unsafe` block does not
inherit the permission: its body is a function value that can be called from
anywhere, so it asks for its own.

What `unsafe` does say is that the compiler stopped proving one thing — that
the address points at anything at all. Nothing checks that a `(Ptr u16)` is
aligned, mapped, or yours. The optimizer will not fold two volatile reads into
one, drop a read whose result is unused, or move one past another; but there is
no memory barrier under any of this, so ordering against *other* accesses is
the program's own business.

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

The type vocabulary is closed. A parameter is any integer type, `f64`, `bool`,
`(Ptr T)`, `(& String)`, or `(& (Slice T))`. A return is `unit`, one of those
scalars, `(Ptr T)`, or an owned `String`. Anything else is refused at the
declaration, and an `extern` cannot be generic: there is nothing a type
parameter could stand for.

A `(Ptr T)` is C's `T *`. Receiving one needs no `unsafe` — a pointer is an
ordinary value until something reads through it.

An `extern` borrows and never moves. A `(& String)` arrives as a
NUL-terminated `const char *`; a `(& (Slice T))` arrives as two arguments, the
element pointer and then the element count. The borrow ends when the call
returns, so C must copy anything it intends to keep. To return a `String`, C
allocates one with the runtime's `sl_rt_string_new(const char *, uint64_t)`;
the caller owns it and drops it as it would any other.

Two things the vocabulary does not say for you. C's `size_t` has no Slopium
spelling and is declared as `u64` on the targets that exist; the fixed widths
line up with `stdint.h` one for one, so an `unsigned char` is a `u8` and an
`int` is an `i32`, and a `long` is whatever the platform made it. And a variadic C function may not be declared at all — the System V ABI
wants the vector-register count in `al` for one, and nothing here sets it.

The C itself is the package's, listed in `Slopium.toml`:

```toml
[package]
c-sources = ["c/hal.c"]
```

Those paths are relative to the package root and may not leave it. They are
compiled with the same `cc` the link uses, ship in the package archive, and
their contents are part of the build cache key, so editing one rebuilds.

The path is checked and the extension is not, so a `.s` is handed to `cc` and
assembled like anything else. That is how a freestanding program supplies the
`_start` the compiler no longer emits for it.
