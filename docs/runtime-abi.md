# Runtime ABI

Generated programs reach the C runtime through `sl_rt_*` symbols, and this
document is that ABI, at **version 1**: every symbol a program can reach, its C
signature, and who owns what across each call. Every `D-nnn` cited below is an
entry in [`decisions.md`](decisions.md), the project's decision log.

The rule the version number carries (`D-108`): **a later minor version may add
symbols, and may not change or remove one.** A signature, an ownership rule, a
layout and a storage class written here hold for as long as the toolchain does.
The list is additive rather than closed because what is absent is known to be
absent — threads, atomics — and what those need is permission to grow, not
permission to move what already links.

The list is kept honest by `scripts/runtime-check.sh`: it compiles each runtime
flavor, reads the defined `sl_`-prefixed symbols out of the object with `nm`,
and compares them against the fenced blocks below, both ways. A symbol the
runtime exports that this document does not list fails the check, and so does a
symbol listed here that the runtime does not export. The fenced blocks are
therefore normative, and every declaration in them keeps its name and opening
parenthesis on one line, which is the shape the check parses; the prose around
them says what a symbol list cannot.

## Conventions

A string crosses the boundary as a pointer and a length, never as a
NUL-terminated pointer alone, because a Slopium string may contain a NUL byte
(`D-079`). The runtime's own string buffer is NUL-terminated anyway, one byte
past `len`, as a courtesy to C code that reads it.

`SlString`, `SlList` and `SlSlice` are heap blocks the runtime allocates and
frees. Generated code holds them by pointer and reads into them directly — the
length at offset 0, the data pointer at offset 16 in a string or a slice and at
offset 24 in a list — so the layouts are part of the ABI, not an implementation
detail of `runtime/slop_rt_core.c`:

```c
typedef struct SlString {
    uint64_t len;
    uint64_t cap;
    char *ptr;
} SlString;

typedef struct SlList {
    uint64_t len;
    uint64_t cap;
    uint64_t elem_size;
    unsigned char *ptr;
    void (*drop_element)(void *);
    uint64_t (*clone_element)(uint64_t);
} SlList;

typedef struct SlSlice {
    uint64_t len;
    uint64_t elem_size;
    unsigned char *ptr;
} SlSlice;
```

A list element is one machine word wide as a value — an integer, or a pointer
to what the element owns (`D-107`) — and `elem_size` is its stored width. The
element helpers are `void (*)(void *)` and `uint64_t (*)(uint64_t)`, and either
may be null when elements own nothing.

A function value is a heap block the compiler lays out (`D-101`): the code
address at word 0, a drop helper `void (*)(void *)` at word 1, a clone helper
`void *(*)(const void *)` at word 2, then one word per capture. The two closure
symbols below dispatch through words 1 and 2, so that layout is ABI too.

A runtime failure — an index out of bounds, an allocation refused — is a panic,
not an error value: the hosted runtime prints `slopium runtime error:` and the
message and exits with status 101, and a freestanding one does what its hooks
say. Hosted entry points that can fail *recoverably* report through the error
slot instead (`D-085`), described with `sl_rt_last_error` below.

## The core runtime

`runtime/slop_rt_core.c` is the half a freestanding program can have
(`D-066`): strings, lists, slices, function values, the float bit casts, and
the failure paths. It exports the symbols of this section and no others, and
leaves nothing undefined but the hooks of the next one. The compiler emits
calls to the string, list, slice and closure helpers itself; the raw-byte
string primitives and the float pair are reached through `extern` declarations
in the bundled `core` library (`D-083`, `D-097`).

### Strings

```c
SlString *sl_rt_string_new(const char *bytes, uint64_t len);
uint64_t sl_rt_string_len(const SlString *string);
SlString *sl_rt_string_clone(const SlString *source);
void sl_rt_string_drop(SlString *string);
int64_t sl_rt_string_byte(const char *bytes, int64_t len, int64_t index);
SlString *sl_rt_string_slice(const char *bytes, int64_t len, int64_t start,
                             int64_t end);
SlString *sl_rt_string_concat(const char *left, int64_t left_len,
                              const char *right, int64_t right_len);
SlString *sl_rt_string_from_bytes(const int64_t *bytes, uint64_t count);
```

Every `SlString *` that comes out is a fresh allocation the caller owns and
eventually hands to `sl_rt_string_drop`, which takes ownership and accepts null
as a no-op. Every pointer that goes in is borrowed for the call, copied from,
and still the caller's afterwards. The four primitives over raw bytes are what
`core:string` cannot write for itself; `sl_rt_string_from_bytes` takes one
`int64_t` per byte, because the library builds bytes in a `(List i64)`, and
keeps the low eight bits of each. An index or range outside the length is a
panic, not an error value.

### Function values

```c
void sl_rt_closure_drop(void *closure);
void *sl_rt_closure_clone(const void *closure);
```

Both dispatch through the helpers inside the block, because the static type
says `Fn` and cannot say which closure. `sl_rt_closure_drop` consumes the block
and everything its captures own, and accepts null as a no-op;
`sl_rt_closure_clone` borrows the block and returns a new one the caller owns.

### Lists

```c
SlList *sl_rt_list_new(uint64_t elem_size, void (*drop_element)(void *),
                       uint64_t (*clone_element)(uint64_t));
void sl_rt_list_push(SlList *list, const void *element);
uint64_t sl_rt_list_len(const SlList *list);
void *sl_rt_list_get(const SlList *list, uint64_t index);
uint64_t sl_rt_list_try_pop(SlList *list, uint64_t *output);
uint64_t sl_rt_list_remove(SlList *list, uint64_t index);
uint64_t sl_rt_list_replace(SlList *list, uint64_t index, const void *element);
SlList *sl_rt_list_clone(const SlList *source);
void sl_rt_list_drop(SlList *list);
```

A list owns its elements. `sl_rt_list_new` returns an empty list the caller
owns; `sl_rt_list_push` copies `elem_size` bytes in and the list owns the
element from then on. `sl_rt_list_try_pop` and `sl_rt_list_remove` hand an
element out as one word the caller now owns — `sl_rt_list_try_pop` answers 0
for an empty list and 1 with the element left in `*output` — and
`sl_rt_list_replace` does both at once: the new element becomes the list's and
the old one becomes the caller's, so exactly one of them is the caller's before
the call and after it (`D-103`). `sl_rt_list_get` returns a pointer into the
list's own storage: a borrow, never freed by the holder, and invalid once the
list grows or drops. `sl_rt_list_clone` copies deeply through `clone_element`;
`sl_rt_list_drop` consumes the list, releasing each element through
`drop_element`, and accepts null as a no-op.

### Slices

```c
SlSlice *sl_rt_slice_new(const SlList *source, uint64_t start, uint64_t end);
SlSlice *sl_rt_slice_clone(const SlSlice *source);
uint64_t sl_rt_slice_len(const SlSlice *slice);
void *sl_rt_slice_get(const SlSlice *slice, uint64_t index);
void sl_rt_slice_drop(SlSlice *slice);
```

A slice is a borrowed view: the caller owns the descriptor and never the
elements, `sl_rt_slice_drop` frees the descriptor alone, and the descriptor is
valid only while the list it views is alive and un-grown. `sl_rt_slice_clone`
copies the descriptor, not the elements. `sl_rt_slice_get` returns a pointer
into the viewed storage, borrowed under the same terms as the descriptor.

### Floats

```c
int64_t sl_rt_f64_bits(double value);
double sl_rt_f64_from_bits(int64_t bits);
```

Bit reinterpretation of one machine word and nothing else: nothing is owned,
nothing rounds, no floating-point flag is read (`D-097`). Every digit of a
decimal expansion is computed above these, in `core:float`.

## The hooks

The core runtime calls four symbols and defines none of them (`D-080`). The
hosted runtime defines all four over libc; a freestanding program supplies its
own, which is the whole seam between a hosted build and a kernel.

```c
void *sl_rt_alloc(uint64_t size);
void sl_rt_free(void *memory);
_Noreturn void sl_rt_abort(void);
_Noreturn void sl_rt_panic(const char *message);
```

`sl_rt_alloc` may return null — core turns a null for a non-zero size into a
panic, so the hook does not have to. `sl_rt_free` must accept null. The message
handed to `sl_rt_panic` is a NUL-terminated static string, borrowed for the
call and never freed. A build with `panic = "abort"` routes every failure to
`sl_rt_abort` and never references `sl_rt_panic`, so such a program supplies
three; the check accordingly holds core's undefined symbols to a subset of
these four, and `scripts/core-check.sh` proves the seam by linking against
them with `-nostdlib`.

## The hosted runtime

`runtime/slop_rt_hosted.c` defines the four hooks above — over `malloc`,
`free`, `exit` and `fprintf` — and adds the symbols of this section. The
bundled `std` library reaches them through `extern` declarations, apart from
`sl_rt_args_init` and `sl_rt_test_result`, which the generated entry wrapper
and test harness call directly. A freestanding build links none of them:
everything below exists only where libc does.

### The error slot

```c
int64_t sl_rt_last_error(void);
```

Every entry point below that can fail recoverably clears a slot on the way in
and sets it on the way out, and the standard library reads it in the form
immediately after the call and builds an `Option` or a `Result` from what it
finds (`D-085`). Zero is success, a positive value is an `errno`, and `-1` is
end of input. The slot is thread-local (`D-108`): threads arrive after 1.0,
and a symbol's storage class cannot move once the ABI holds, so it moved
first — each thread already reads only what its own calls left.

### Standard streams

```c
void sl_rt_println_bytes(const char *bytes, int64_t len);
void sl_rt_print_bytes(const char *bytes, int64_t len);
SlString *sl_rt_read_line(void);
```

The bytes are borrowed for the call and written whole, embedded NUL bytes
included (`D-079`). `sl_rt_read_line` returns a string the caller owns; at end
of input it returns an empty string and sets the slot to `-1`, because an
empty line is a value and the slot is what tells them apart (`D-087`).

### Process and environment

```c
SlString *sl_rt_env(const char *name, int64_t len);
_Noreturn void sl_rt_exit(int64_t code);
void sl_rt_args_init(int32_t argc, char **argv);
int64_t sl_rt_args_len(void);
SlString *sl_rt_arg(int64_t index);
```

`sl_rt_args_init` is called once by the generated entry wrapper, before user
code runs, and holds the vector without copying — the vector C hands `main`
outlives the program, and nothing else may be passed. `sl_rt_arg` returns an
owned copy; index 0 is the first argument after the program's own name, and an
index outside `sl_rt_args_len` is a panic, which is why the library checks
first. `sl_rt_env` borrows the name and returns an owned copy of the value,
setting the slot to `ENOENT` for a variable that is not set and `EINVAL` for a
name containing a NUL byte.

### Files

```c
SlString *sl_rt_fs_read(const char *path, int64_t len);
int64_t sl_rt_fs_write(const char *path, int64_t path_len, const char *data,
                       int64_t data_len);
int64_t sl_rt_fs_exists(const char *path, int64_t len);
int64_t sl_rt_fs_remove(const char *path, int64_t len);
```

A file is read and written whole, because a handle has no destructor
(`D-084`). Paths and data are borrowed for the call; `sl_rt_fs_read` returns a
string the caller owns, empty with the slot set when the read failed.
`sl_rt_fs_write` returns the bytes written, or `-1` with the slot set;
`sl_rt_fs_exists` answers 1 or 0, and `sl_rt_fs_remove` answers 0, or `-1`
with the slot set. A path containing a NUL byte names no file and is `EINVAL`,
never a truncation (`D-079`).

### The test harness

```c
void sl_rt_test_note(const char *message);
int32_t sl_rt_test_result(const char *name, int32_t passed);
```

`sl_rt_test_note` copies the message into a bounded internal buffer, so the
string it was read from is still the caller's to drop (`D-130`).
`sl_rt_test_result` borrows the name, prints the verdict with the pending note
when the test failed, clears the note, and returns 0 for a pass and 1 for a
failure.

### Time and randomness

```c
int64_t sl_rt_time_monotonic(void);
int64_t sl_rt_time_realtime(void);
int64_t sl_rt_random_bytes(int64_t *elements, int64_t count);
```

A clock reading is one `int64_t` of nanoseconds (`D-147`), zero with the slot
set when the clock failed. `sl_rt_random_bytes` fills a buffer the caller
already sized and may not resize (`D-124`), one word per byte (`D-107`), and
returns how many bytes it wrote, which the library compares against what it
asked for.

### Child processes

```c
int64_t sl_rt_process_spawn(const char *program, int64_t program_len,
                            const char *arguments, int64_t arguments_len,
                            int64_t capture, int64_t *output);
int64_t sl_rt_process_wait(int64_t pid);
SlString *sl_rt_process_read(int64_t descriptor);
int64_t sl_rt_process_close(int64_t descriptor);
```

The program name and the argument vector — one buffer of NUL-separated pieces
with its length beside it — are borrowed for the call (`D-148`).
`sl_rt_process_spawn` returns the child's pid, or 0 with the slot set; a
non-zero `capture` hands the read end of a pipe back through `output`, and
that descriptor is the caller's to close with `sl_rt_process_close`, while
`-1` there means nothing was opened and close accepts it as a no-op.
`sl_rt_process_wait` returns the exit status, `128` plus the signal for a
child something killed, or `-1` with the slot saying why the wait itself
failed. `sl_rt_process_read` drains the descriptor to end of input and returns
a string the caller owns.
