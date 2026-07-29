//! End-to-end tests exercising the built `sysml-v2-cli` binary.

use std::io::Write;
use std::process::Command;

fn run(source: &str) -> (bool, String) {
    let mut file = tempfile::Builder::new()
        .suffix(".sysml")
        .tempfile()
        .unwrap();
    write!(file, "{source}").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("lint")
        .arg(file.path())
        .output()
        .expect("failed to run sysml-v2-cli");

    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn accepts_a_valid_package() {
    let (ok, _) = run(r#"
        package Foo {
            attribute def X { attribute a : String; }
        }
        "#);
    assert!(ok);
}

#[test]
fn rejects_state_as_a_plain_identifier_case() {
    // Regression case: syster (syster-base 0.4.3-alpha) failed to parse
    // `out item state : T;` as a port member, treating `state` as a keyword.
    // sysml-v2-parser must accept it.
    let (ok, _) = run(r#"
        package Foo {
            item def T;
            port def P { out item state : T; }
        }
        "#);
    assert!(
        ok,
        "sysml-v2-parser should accept `state` as an ordinary identifier"
    );
}

#[test]
fn flags_a_missing_closing_brace() {
    let (ok, stdout) = run(r#"
        package Foo {
            attribute def X { attribute a : String; }
        "#);
    assert!(!ok);
    assert!(stdout.contains("missing closing"));
}

#[test]
fn same_line_inline_redefinition_body_parses() {
    // Was a known limitation: sysml-v2-parser 0.29.0 mis-tracked brace nesting
    // when a usage's inline redefinition body closed its '}' on the same line as
    // the last statement inside it, so models had to split the closing brace onto
    // its own line. Fixed as of the 0.48.0 upgrade — this test now guards the fix
    // rather than documenting the bug, and the multi-line form below must keep
    // working too.
    let (ok_inline, _) = run(r#"
        package Foo {
            port def P { attribute address : String; }
            part X {
                port cameraOut : P { attribute address : String = "tcp://*:5555"; }
            }
        }
        "#);
    assert!(
        ok_inline,
        "same-line inline redefinition body should parse since 0.48.0"
    );

    let (ok_multiline, _) = run(r#"
        package Foo {
            port def P { attribute address : String; }
            part X {
                port cameraOut : P {
                    attribute address : String = "tcp://*:5555";
                }
            }
        }
        "#);
    assert!(ok_multiline);
}

#[test]
fn strict_and_edit_modes_disagree_about_allocate() {
    // `allocate` in a part-definition body is valid SysML v2 and the strict
    // parser accepts it; the error-recovery parser does not implement that
    // production and reports it. The mode flag exists because both answers are
    // useful: strict for validating a model, edit for seeing what
    // recovery-based tooling (an LSP) will say.
    let src = r#"
        package Foo {
            part def A { attribute x : String; }
            part def B {
                ref part image : A;
                action step;
                allocate action step to image;
            }
        }
        "#;

    let (ok_strict, errors_strict) = sysml_v2_cli::lint::lint_source_mode(
        src,
        sysml_v2_cli::lint::LintMode::Strict,
    );
    assert!(ok_strict, "strict parser should accept `allocate`");
    assert!(errors_strict.is_empty());

    let (ok_edit, errors_edit) =
        sysml_v2_cli::lint::lint_source_mode(src, sysml_v2_cli::lint::LintMode::Edit);
    assert!(!ok_edit, "recovery parser does not cover `allocate` yet");
    assert!(!errors_edit.is_empty());

    // Default is strict.
    let (ok_default, _) = sysml_v2_cli::lint::lint_source(src);
    assert_eq!(ok_default, ok_strict);
}

#[test]
fn both_modes_reject_a_real_syntax_error() {
    let src = "package Foo {\n    attribute def X { attribute a : String;\n";
    for mode in [
        sysml_v2_cli::lint::LintMode::Strict,
        sysml_v2_cli::lint::LintMode::Edit,
    ] {
        let (ok, errors) = sysml_v2_cli::lint::lint_source_mode(src, mode);
        assert!(!ok, "{mode:?} should reject a missing closing brace");
        assert!(!errors.is_empty(), "{mode:?} should explain the failure");
    }
}
