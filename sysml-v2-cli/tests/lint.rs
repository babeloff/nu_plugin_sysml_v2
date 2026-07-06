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
fn same_line_inline_redefinition_body_is_a_known_parser_limitation() {
    // sysml-v2-parser 0.29.0 mis-tracks brace nesting when a usage's inline
    // redefinition body has its closing '}' on the same line as the last
    // statement inside it. Splitting the closing brace onto its own line
    // works around it. This test documents the limitation so an upstream
    // fix (or a version bump) is visible as a test failure here, not a
    // silent behavior change.
    let (ok_inline, _) = run(r#"
        package Foo {
            port def P { attribute address : String; }
            part X {
                port cameraOut : P { attribute address : String = "tcp://*:5555"; }
            }
        }
        "#);
    assert!(
        !ok_inline,
        "if this now passes, the upstream bug is fixed — simplify resources.sysml formatting"
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
