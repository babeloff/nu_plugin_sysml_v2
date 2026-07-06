//! End-to-end tests for `sysml-v2-cli emit`, moved here from
//! `channel-adapter/crate/sysml-emit`.

use std::io::Write;
use std::process::Command;

#[test]
fn emits_proto_and_xsd_to_stdout() {
    let mut file = tempfile::Builder::new()
        .suffix(".sysml")
        .tempfile()
        .unwrap();
    write!(
        file,
        r#"
        package CotDetailMsg {{
            item def Track {{
                attribute course : Real;
                attribute speed  : Real;
            }}
        }}
        "#
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("emit")
        .arg(file.path())
        .output()
        .expect("failed to run sysml-v2-cli emit");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("syntax = \"proto3\";"));
    assert!(stdout.contains("message Track"));
    assert!(stdout.contains("<?xml version=\"1.0\""));
    assert!(stdout.contains("complexType"));
}

#[test]
fn emits_proto_and_xsd_to_directories() {
    let mut file = tempfile::Builder::new()
        .suffix(".sysml")
        .tempfile()
        .unwrap();
    write!(
        file,
        r#"
        package CotDetailMsg {{
            item def Track {{
                attribute course : Real;
            }}
        }}
        "#
    )
    .unwrap();

    let out_dir = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sysml-v2-cli"))
        .arg("emit")
        .arg(file.path())
        .arg("--proto-dir")
        .arg(out_dir.path())
        .arg("--xsd-dir")
        .arg(out_dir.path())
        .output()
        .expect("failed to run sysml-v2-cli emit");

    assert!(output.status.success());

    let stem = file.path().file_stem().unwrap().to_str().unwrap();
    let proto_path = out_dir.path().join(format!("{stem}.proto"));
    let xsd_path = out_dir.path().join(format!("{stem}.xsd"));
    assert!(
        proto_path.exists(),
        "expected {} to exist",
        proto_path.display()
    );
    assert!(
        xsd_path.exists(),
        "expected {} to exist",
        xsd_path.display()
    );
}
