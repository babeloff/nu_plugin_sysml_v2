//! End-to-end tests for `sysml-v2-cli lint --resolve-imports` and
//! `sysml-v2-cli emit --resolve-imports`, exercising the built binary
//! against a synthetic ISQ/SI-style library directory.

use std::fs;
use std::process::Command;

/// A temp directory with `SI.sysml` (defines `SI::kg`) and `ISQ.sysml`
/// (imports `SI::*`, defines `ISQ::MassValue :> SI::kg`).
fn isq_si_lib_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("SI.sysml"),
        "standard library package SI {\n    attribute def kg;\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ISQ.sysml"),
        "standard library package ISQ {\n    private import SI::*;\n    attribute def MassValue :> SI::kg;\n}\n",
    )
    .unwrap();
    dir
}

fn write_sysml(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn resolves_isq_reference_successfully() {
    let lib_dir = isq_si_lib_dir();
    let scratch = tempfile::tempdir().unwrap();
    let file = write_sysml(
        scratch.path(),
        "good.sysml",
        r#"
        package Weighing {
            private import ISQ::*;
            item def Scale {
                attribute mass : ISQ::MassValue;
            }
        }
        "#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("lint")
        .arg("--resolve-imports")
        .arg("--lib-dir")
        .arg(lib_dir.path())
        .arg(&file)
        .output()
        .expect("failed to run sysml-v2-cli");

    assert!(
        output.status.success(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn flags_unresolved_import_and_reference() {
    let lib_dir = isq_si_lib_dir();
    let scratch = tempfile::tempdir().unwrap();
    let file = write_sysml(
        scratch.path(),
        "bad.sysml",
        r#"
        package Weighing {
            private import ISQ::*;
            private import NoSuchPackage::*;
            item def Scale {
                attribute mass : ISQ::NoSuchQuantity;
            }
        }
        "#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("lint")
        .arg("--json")
        .arg("--resolve-imports")
        .arg("--lib-dir")
        .arg(lib_dir.path())
        .arg(&file)
        .output()
        .expect("failed to run sysml-v2-cli");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unresolved import: NoSuchPackage::*"), "{stdout}");
    assert!(
        stdout.contains("unresolved reference to ISQ::NoSuchQuantity"),
        "{stdout}"
    );
}

#[test]
fn without_the_flag_unresolved_imports_are_not_flagged() {
    let scratch = tempfile::tempdir().unwrap();
    let file = write_sysml(
        scratch.path(),
        "unflagged.sysml",
        r#"
        package Weighing {
            private import ISQ::*;
            item def Scale {
                attribute mass : ISQ::MassValue;
            }
        }
        "#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("lint")
        .arg(&file)
        .output()
        .expect("failed to run sysml-v2-cli");

    assert!(
        output.status.success(),
        "plain lint (no --resolve-imports) must stay syntax-only"
    );
}

#[test]
fn emit_derives_cross_package_import_from_resolved_library_file() {
    let lib_dir = isq_si_lib_dir();
    fs::write(
        lib_dir.path().join("CotEventMsg.sysml"),
        "package CotEventMsg {\n    item def CotEvent {\n        attribute uid : String;\n    }\n}\n",
    )
    .unwrap();

    let scratch = tempfile::tempdir().unwrap();
    let file = write_sysml(
        scratch.path(),
        "cross-pkg.sysml",
        r#"
        package CotDetailMsg {
            private import CotEventMsg::*;
            item def Detail {
                attribute event : CotEventMsg::CotEvent;
            }
        }
        "#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("emit")
        .arg("--resolve-imports")
        .arg("--lib-dir")
        .arg(lib_dir.path())
        .arg(&file)
        .output()
        .expect("failed to run sysml-v2-cli");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Derived from CotEventMsg.sysml's own filename stem, not the
    // hardcoded granule package table's "cotevent.proto".
    assert!(stdout.contains(r#"import "CotEventMsg.proto";"#), "{stdout}");
}
