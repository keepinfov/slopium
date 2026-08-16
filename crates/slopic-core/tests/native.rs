use slopic_core::codegen::{emit_assembly, CodegenOptions, DEFAULT_TARGET};
use slopic_core::{
    compile, compile_to_mir, CompileOptions, CompileRequest, DependencySource, EmitKind,
    LanguageItems, STD_PACKAGE,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);

/// I/O is a library now, so every program below has to say so. These tests are
/// about what the backends emit, not about naming imports, so the whole surface
/// is taken once here and the sources stay about their subject.
const TAKES: &str = "\
(take std:io print println print-i64 println-i64 print-bool println-bool
  read-line read-i64)
(take std:string to-i64 trim)
(take std:process env args-len arg)
";

/// The bundled runtime written into `directory`, in link order. A hosted test
/// links both halves, the way `slopic` does (`D-066`).
fn write_runtime(directory: &Path) -> Vec<PathBuf> {
    slopic_core::runtime_sources(slopic_core::codegen::Environment::Hosted)
        .into_iter()
        .map(|(name, bytes)| {
            let path = directory.join(name);
            fs::write(&path, bytes).unwrap();
            path
        })
        .collect()
}

fn native_program(source: &str) -> (PathBuf, PathBuf) {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-native-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("main.slp");
    let output = directory.join("program");
    fs::write(&input, format!("{TAKES}{source}")).unwrap();
    compile(&CompileRequest {
        input,
        source_root: None,
        dependencies: Vec::new(),
        toolchain_dependencies: vec![STD_PACKAGE.to_owned()],
        output: Some(output.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtimes: Vec::new(),
        cc: "cc".into(),
    })
    .unwrap();
    (directory, output)
}

#[test]
fn compiles_and_runs_recursive_native_program() {
    let (directory, executable) = native_program(
        r#"
        (fn fib ((n i64)) -> i64
          (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
        (fn main () -> i32
          (println-i64 (fib 10))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "55\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compiles_and_runs_multi_file_package() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("slopic-package-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let input = source_root.join("main.slp");
    let output = directory.join("program");
    fs::write(
        source_root.join("geometry.slp"),
        "(export distance)\n(fn distance ((n i64)) -> i64 (+ n 1))\n",
    )
    .unwrap();
    fs::write(
        &input,
        format!(
            "{TAKES}(take geometry (distance :as length))\n\
             (fn main () -> i32\n\
               (println-i64 (length 41))\n\
               (println-i64 (geometry:distance 9))\n\
               0)\n"
        ),
    )
    .unwrap();
    compile(&CompileRequest {
        input,
        source_root: Some(source_root),
        dependencies: Vec::new(),
        toolchain_dependencies: vec![STD_PACKAGE.to_owned()],
        output: Some(output.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtimes: Vec::new(),
        cc: "cc".into(),
    })
    .unwrap();
    let process = Command::new(output).output().unwrap();
    assert!(process.status.success());
    assert_eq!(String::from_utf8(process.stdout).unwrap(), "42\n10\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compiles_path_dependency_under_manifest_alias() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("slopic-dependency-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    let app = directory.join("app");
    let dependency = directory.join("dependency");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&dependency).unwrap();
    fs::write(
        dependency.join("ops.slp"),
        "(export double)\n(fn double ((value i64)) -> i64 (* value 2))\n",
    )
    .unwrap();
    let input = app.join("main.slp");
    fs::write(
        &input,
        format!("{TAKES}(take math:ops double)\n(fn main () -> i32 (println-i64 (double 21)) 0)\n"),
    )
    .unwrap();
    let executable = directory.join("program");
    compile(&CompileRequest {
        input,
        source_root: Some(app),
        dependencies: vec![DependencySource {
            namespace: "math".into(),
            source_root: dependency,
        }],
        toolchain_dependencies: vec![STD_PACKAGE.to_owned()],
        output: Some(executable.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtimes: Vec::new(),
        cc: "cc".into(),
    })
    .unwrap();
    let process = Command::new(executable).output().unwrap();
    assert!(process.status.success());
    assert_eq!(String::from_utf8(process.stdout).unwrap(), "42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn monomorphizes_parametric_generic_functions() {
    let (directory, executable) = native_program(
        r#"
        (fn identity (T) ((value T)) -> T value)
        (fn main () -> i32
          (println-i64 (identity 42))
          (let text (identity "generic"))
          (println (& text))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\ngeneric\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn monomorphizes_generic_structs_and_enums() {
    let (directory, executable) = native_program(
        r#"
        (struct Box (T) ((value T)))
        (enum Option (T) None (Some ((value T))))
        (fn wrap (T) ((value T)) -> (Box T)
          (Box :value value))

        (fn main () -> i32
          (let wrapped (wrap 1))
          (let boxed (Box :value 2))
          (println-i64 (. boxed value))
          (let option (Option:Some 40))
          (println-i64
            (match option
              ((Option:None) 0)
              ((Option:Some value) value)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "2\n40\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn try_propagates_configured_result_errors() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-try-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("main.slp");
    let executable = directory.join("program");
    fs::write(
        &input,
        r#"
        (take std:io println println-i64)

        (export Result (Result:Ok :as Ok) (Result:Err :as Err))

        (enum Result (T E)
          (Ok ((value T)))
          (Err ((error E))))

        (fn produce ((ok bool)) -> (Result i64 String)
          (if ok
              (Result:Ok 41)
              (Result:Err "failure")))

        (fn forward ((ok bool)) -> (Result i64 String)
          (let value (try (produce ok)))
          (Result:Ok (+ value 1)))

        (fn show ((result (Result i64 String))) -> i64
          (match result
            ((Result:Ok value) value)
            ((Result:Err error)
              (do (println (& error)) 0))))

        (fn main () -> i32
          (println-i64 (show (forward true)))
          (println-i64 (show (forward false)))
          0)
        "#,
    )
    .unwrap();
    compile(&CompileRequest {
        input,
        source_root: None,
        dependencies: Vec::new(),
        toolchain_dependencies: vec![STD_PACKAGE.to_owned()],
        output: Some(executable.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions {
            language_items: LanguageItems {
                option: None,
                // A lone file is a package of one module now (`D-077`), so a
                // language item in it is named the way any other declaration is.
                result: Some("main:Result".into()),
                result_ok: Some("main:Ok".into()),
                result_err: Some("main:Err".into()),
            },
            ..CompileOptions::default()
        },
        runtimes: Vec::new(),
        cc: "cc".into(),
    })
    .unwrap();
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "42\nfailure\n0\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn toolchain_std_supplies_option_result_and_language_items() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-std-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let input = source_root.join("main.slp");
    let executable = directory.join("program");
    fs::write(
        &input,
        r#"
        (take std:io println-i64)
        (take std:prelude Result Option)

        (fn produce () -> (Result i64 String)
          (Result:Ok 42))

        (fn forward () -> (Result i64 String)
          (let value (try (produce)))
          (Result:Ok value))

        (fn main () -> i32
          (let mut values (list 7))
          (println-i64
            (match (pop (&mut values))
              ((Option:Some value) value)
              ((Option:None) -1)))
          (println-i64
            (match (pop (&mut values))
              ((Option:Some value) value)
              ((Option:None) 0)))
          (println-i64
            (match (forward)
              ((Result:Ok value) value)
              ((Result:Err error) 0)))
          0)
        "#,
    )
    .unwrap();
    compile(&CompileRequest {
        input,
        source_root: Some(source_root),
        dependencies: Vec::new(),
        toolchain_dependencies: vec!["std".into()],
        output: Some(executable.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtimes: Vec::new(),
        cc: "cc".into(),
    })
    .unwrap();
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "7\n0\n42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn strings_lists_structs_and_tests_use_the_runtime() {
    let (directory, executable) = native_program(
        r#"
        (struct Point ((x i64) (y i64)))
        (fn main () -> i32
          (let message "native")
          (println (& message))
          (let point (Point :x 20 :y 22))
          (let mut values (list (. point x) (. point y)))
          (do (push (&mut values) 10))
          (println-i64 (+ (get (& values) 0) (get (& values) 1)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "native\n42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn enum_match_transfers_owned_payload() {
    let (directory, executable) = native_program(
        r#"
        (enum Message Empty (Text ((value String))))
        (fn consume ((message Message)) -> i64
          (match message
            ((Message:Empty) 0)
            ((Message:Text value)
              (do (println (& value)) 42))))
        (fn main () -> i32
          (println-i64 (consume (Message:Text "payload")))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "payload\n42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reads_integer_and_returns_owned_environment_string() {
    let (directory, executable) = native_program(
        r#"
        (take std:prelude Option)
        (fn main () -> i32
          (let expected (read-i64))
          (let key "SLOPIUM_NATIVE_TEST_FLAG")
          (if (match expected ((Option:Some value) (= value 1337)) ((Option:None) false))
              (match (env (& key))
                ((Option:Some flag) (do (println (& flag)) 0))
                ((Option:None) 1))
              1))
        "#,
    );
    let mut child = Command::new(executable)
        .env("SLOPIUM_NATIVE_TEST_FLAG", "slopium{runtime_io_works}")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"1337\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "slopium{runtime_io_works}\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reads_lines_parses_numbers_and_exposes_process_arguments() {
    let (directory, executable) = native_program(
        r#"
        (take std:prelude Option)
        (fn main () -> i32
          (let parsed
            (match (read-line)
              ((Option:Some line) (do (let text (trim (& line))) (to-i64 (& text))))
              ((Option:None) (Option:None))))
          (println-i64 (match parsed ((Option:Some value) value) ((Option:None) 0)))
          (println-i64 (args-len))
          (match (arg 0)
            ((Option:Some first) (println (& first)))
            ((Option:None) ()))
          0)
        "#,
    );
    let mut child = Command::new(executable)
        .arg("hello")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b" 42\r\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n1\nhello\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn runtime_errors_use_stable_message_and_exit_status() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let values (list 1 2 3))
          (println-i64 (get (& values) 7))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "slopium runtime error: list index out of bounds\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn list_owns_and_moves_non_copy_elements() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let mut values (list "first" "second" "third"))
          (let first (get-ref (& values) 0))
          (println first)
          (let removed (remove (&mut values) 1))
          (println (& removed))
          (let last (remove (&mut values) 1))
          (println (& last))
          (println-i64 (len (& values)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "first\nsecond\nthird\n1\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn clone_is_structural_for_lists_structs_and_enums() {
    let (directory, executable) = native_program(
        r#"
        (struct Payload ((label String) (values (List String))))
        (enum Message
          Empty
          (Data ((payload Payload))))
        (fn main () -> i32
          (let original
            (Message:Data
              (Payload :label "owned" :values (list "one" "two"))))
          (let copied (clone original))
          (println-i64 42)
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loops_support_break_continue_and_mutation() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let mut n 0)
          (while (< n 5)
            (set n (+ n 1))
            (if (= n 2) (continue) ())
            (if (= n 4) (break) ())
            (println-i64 n))
          (println-i64 n)
          (loop
            (set n (+ n 1))
            (if (= n 6) (break) ()))
          (println-i64 n)
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "1\n3\n4\n6\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn arrays_and_borrowed_slices_support_owned_elements() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let values (array "zero" "one" "two" "three"))
          (let view (slice (& values) 1 3))
          (println-i64 (len (& view)))
          (let first (get-ref (& view) 0))
          (println first)
          (let copied (clone values))
          (let last (get-ref (& copied) 3))
          (println last)
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "2\none\nthree\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn nested_enum_and_struct_patterns_destructure_owned_values() {
    let (directory, executable) = native_program(
        r#"
        (struct Point ((x i64) (label String)))
        (enum Inner
          None
          (Value ((number i64))))
        (enum Outer
          Empty
          (Wrap ((inner Inner)))
          (Pointed ((point Point))))
        (fn score ((value Outer)) -> i64
          (match value
            ((Outer:Wrap (Inner:Value number)) number)
            ((Outer:Wrap (Inner:None)) 0)
            ((Outer:Pointed (Point :x x :label label))
              (do (println (& label)) x))
            ((Outer:Empty) -1)
            (_ -2)))
        (fn main () -> i32
          (println-i64 (score (Outer:Wrap (Inner:Value 42))))
          (println-i64 (score (Outer:Pointed (Point :x 7 :label "point"))))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\npoint\n7\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mixed_register_and_stack_arguments_round_trip() {
    let (directory, executable) = native_program(
        r#"
        (fn probe
          ((i0 i64) (f0 f64) (i1 i64) (f1 f64) (i2 i64) (f2 f64)
           (i3 i64) (f3 f64) (i4 i64) (f4 f64) (i5 i64) (f5 f64)
           (i6 i64) (f6 f64) (i7 i64) (f7 f64) (f8 f64))
          -> i64
          (if (= f8 9.0) (+ i6 i7) 0))
        (fn main () -> i32
          (println-i64 (probe 1 1.0 2 2.0 3 3.0 4 4.0 5 5.0 6 6.0
                          7 7.0 8 8.0 9.0))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "15\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_double_result_survives_a_spilled_destination() {
    // Ten double arguments hold enough locals live that the result has nowhere
    // to live but the frame, which is the one case the `xmm0` unload used to
    // assume away.
    let (directory, executable) = native_program(
        r#"
        (fn total ((a f64) (b f64) (c f64) (d f64) (e f64)
                   (f f64) (g f64) (h f64) (i f64) (j f64)) -> f64
          (+ (+ (+ (+ a b) (+ c d)) (+ (+ e f) (+ g h))) (+ i j)))
        (fn main () -> i32
          (let sum (total 1.0 2.0 3.0 4.0 5.0 6.0 7.0 8.0 9.0 10.0))
          (if (= sum 55.0) (println-i64 1) (println-i64 0))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "1\n");
    fs::remove_dir_all(directory).unwrap();
}

/// The reverse of [`c_caller_agrees_with_slopium_stack_parameter_layout`]:
/// Slopium calls C, and the two runtime layout facts `extern_arguments`
/// encodes — a `String`'s pointer and a `Slice`'s pointer and length, read at
/// fixed byte offsets — are checked by C reading them as ordinary arguments.
#[test]
fn an_extern_call_reaches_c_with_the_arguments_it_declared() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-ffi-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let source = r#"
        (extern "probe_println_i32" (println-i32 (value i32)) -> unit)
        (extern "probe_println_i64" (println-i64 (value i64)) -> unit)
        (extern "sl_rt_println_bytes"
          (println-bytes (text (& String)) (length i64)) -> unit)
        (extern "probe_narrow" (probe-narrow (value i32)) -> i32)
        (extern "probe_strlen" (probe-strlen (text (& String))) -> i64)
        (extern "probe_slice" (probe-slice (values (& (Slice i64)))) -> i64)
        (extern "probe_string" (probe-string) -> String)
        (fn main () -> i32
          (println-i32 (probe-narrow 2000000000))
          (let text "borrowed")
          (println-i64 (probe-strlen (& text)))
          (let values (array 10 20 30 40))
          (let view (slice (& values) 1 4))
          (println-i64 (probe-slice (& view)))
          (let greeting (probe-string))
          (println-bytes (& greeting) (len (& greeting)))
          0)
    "#;
    let module = compile_to_mir("ffi.slp", source, &CompileOptions::default()).unwrap();
    let assembly = emit_assembly(
        "ffi.slp",
        &module,
        &CodegenOptions {
            target: DEFAULT_TARGET.into(),
            test_harness: false,
            emit_entrypoint: true,
            debug: None,
            panic_abort: false,
        },
    )
    .unwrap();
    let assembly_path = directory.join("ffi.s");
    let runtime_paths = write_runtime(&directory);
    let callee_path = directory.join("callee.c");
    let executable = directory.join("ffi");
    fs::write(&assembly_path, assembly).unwrap();
    fs::write(
        &callee_path,
        r#"
        #include <stdint.h>
        #include <stdio.h>
        #include <string.h>
        typedef struct { uint64_t len; uint64_t cap; char *ptr; } SlString;
        SlString *sl_rt_string_new(const char *bytes, uint64_t len);

        /* The printers are the probe's own: the runtime prints bytes, and a
         * number reaches it through the library's Slopium formatter (`D-086`),
         * which this test deliberately does not link. */
        void probe_println_i32(int32_t value) { printf("%d\n", value); }
        void probe_println_i64(int64_t value) { printf("%ld\n", (long)value); }

        int32_t probe_narrow(int32_t value) { return -value; }
        int64_t probe_strlen(const char *text) { return (int64_t)strlen(text); }
        int64_t probe_slice(const int64_t *values, int64_t len) {
            int64_t total = 0;
            for (int64_t index = 0; index < len; index++) {
                total += values[index] * (index + 1);
            }
            return total;
        }
        SlString *probe_string(void) { return sl_rt_string_new("from C", 6); }
        "#,
    )
    .unwrap();
    let status = Command::new("cc")
        .arg("-o")
        .arg(&executable)
        .arg(&assembly_path)
        .args(&runtime_paths)
        .arg(&callee_path)
        .status()
        .unwrap();
    assert!(status.success());
    let output = Command::new(&executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "-2000000000\n8\n200\nfrom C\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn c_caller_agrees_with_slopium_stack_parameter_layout() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-c-abi-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let source = r#"
        (fn abi_probe
          ((i0 i64) (f0 f64) (i1 i64) (f1 f64) (i2 i64) (f2 f64)
           (i3 i64) (f3 f64) (i4 i64) (f4 f64) (i5 i64) (f5 f64)
           (i6 i64) (f6 f64) (i7 i64) (f7 f64) (f8 f64))
          -> i64
          (if (= f8 9.0) (+ i6 i7) 0))
        (fn main () -> i32 0)
    "#;
    let module = compile_to_mir("abi.slp", source, &CompileOptions::default()).unwrap();
    let assembly = emit_assembly(
        "abi.slp",
        &module,
        &CodegenOptions {
            target: DEFAULT_TARGET.into(),
            test_harness: false,
            emit_entrypoint: false,
            debug: None,
            panic_abort: false,
        },
    )
    .unwrap();
    let assembly_path = directory.join("abi.s");
    let runtime_paths = write_runtime(&directory);
    let oracle_path = directory.join("oracle.c");
    let executable = directory.join("oracle");
    fs::write(&assembly_path, assembly).unwrap();
    fs::write(
        &oracle_path,
        r#"
        #include <stdint.h>
        extern int64_t abi_probe(
            int64_t, double, int64_t, double, int64_t, double,
            int64_t, double, int64_t, double, int64_t, double,
            int64_t, double, int64_t, double, double
        ) __asm__("sl_fn_6162695f70726f6265");
        int main(void) {
            return abi_probe(
                1, 1.0, 2, 2.0, 3, 3.0, 4, 4.0, 5, 5.0, 6, 6.0,
                7, 7.0, 8, 8.0, 9.0
            ) == 15 ? 0 : 1;
        }
        "#,
    )
    .unwrap();
    let status = Command::new("cc")
        .args(["-o"])
        .arg(&executable)
        .arg(&assembly_path)
        .args(&runtime_paths)
        .arg(&oracle_path)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(Command::new(&executable).status().unwrap().success());
    fs::remove_dir_all(directory).unwrap();
}

/// A raw pointer reaches memory at every width, and reads back what it wrote
/// (`D-067`).
///
/// The buffer comes from the runtime's own allocator, which is already linked,
/// so this needs no C of its own. What it really checks is the narrow loads and
/// stores neither backend had before: a byte and a half are the only memory
/// this compiler touches that is not a machine word.
#[test]
fn a_raw_pointer_reads_back_what_it_wrote_at_every_width() {
    let (directory, executable) = native_program(
        r#"
        (extern "sl_rt_alloc" (rt-alloc (size u64)) -> (Ptr u8))
        (fn main () -> i32
          (unsafe
            (let bytes (rt-alloc 64))
            (volatile-write bytes 0xAB)
            (volatile-write (ptr-offset bytes 1) 0xCD)
            (println-i64 (as i64 (volatile-read bytes)))
            (println-i64 (as i64 (volatile-read (ptr-offset bytes 1))))
            (let signed (as (Ptr i8) bytes))
            (println-i64 (as i64 (volatile-read signed)))
            (let halves (as (Ptr u16) bytes))
            (volatile-write halves 0x1234)
            (println-i64 (as i64 (volatile-read halves)))
            (let words (as (Ptr u32) bytes))
            (volatile-write words 0xDEADBEEF)
            (println-i64 (as i64 (volatile-read words))))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    // `0xAB` has its top bit set, so a sign-extending read of that one byte is
    // -85 where a zero-extending one is 171. Both are here, from the same byte.
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "171\n205\n-85\n4660\n3735928559\n"
    );
    fs::remove_dir_all(directory).unwrap();
}
