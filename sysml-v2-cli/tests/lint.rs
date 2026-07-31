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
fn view_rendering_bodies_carry_nested_members() {
    // Fixed in sysml-v2-parser 0.49.0: `render`/`rendering` usage bodies were
    // opaque, so an anonymous column redefinition nested inside one was parsed
    // as unrecoverable text, and `view :>> name[n] { … }` was not accepted at
    // all. Both forms come from the OMG Systems Library's `asElementTable` /
    // `columnView` mechanism. Both entry points accept them, so one assertion
    // per mode.
    for src in [
        r#"
        package P {
            view def V {
                rendering asTextualNotationTable :> asElementTable {
                    view :>> columnView[1] {
                        render asTextualNotation;
                    }
                }
            }
        }
        "#,
        r#"
        package P {
            view v {
                render asElementTable {
                    view :>> columnView[1] {
                        render asTextualNotation;
                    }
                }
            }
        }
        "#,
    ] {
        for mode in [
            sysml_v2_cli::lint::LintMode::Strict,
            sysml_v2_cli::lint::LintMode::Edit,
        ] {
            let (ok, errors) = sysml_v2_cli::lint::lint_source_mode(src, mode);
            assert!(
                ok,
                "{mode:?} should accept a nested column-view redefinition since \
                 0.49.0, got {errors:?}",
                errors = errors.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn comment_prose_is_not_read_as_a_declaration() {
    // Fixed in sysml-v2-parser 0.50.0 (upstream issue #1, commits ee9b8a9 +
    // f64335d): the recovery parser used to stop treating a comment as a comment
    // when a continuation line read `<identifier>: <text>`, but only inside a
    // *part definition* body, and report the prose as a feature declaration.
    //
    // This is criterion 3 of the now-retired docs/specs/01-fix-comment-block.adoc:
    // both modes must accept the reproduction, for all three comment forms, so a
    // future parser bump that reintroduces the defect fails here rather than
    // silently changing what models may say.
    for open in ["/**", "/*", "doc /*"] {
        let src = format!(
            "package P {{\n\
             \x20   part def C {{\n\
             \x20       {open} first line\n\
             \x20           Optional: a profile may state the rate */\n\
             \x20       attribute x : String;\n\
             \x20   }}\n\
             }}\n"
        );

        for mode in [
            sysml_v2_cli::lint::LintMode::Strict,
            sysml_v2_cli::lint::LintMode::Edit,
        ] {
            let (ok, errors) = sysml_v2_cli::lint::lint_source_mode(&src, mode);
            assert!(
                ok,
                "{mode:?} should accept a `{open} … Optional: … */` comment, got {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn both_modes_reject_malformed_declarations() {
    // Criterion 2 of docs/specs/02-parser-inconsistency.adoc, met in 0.50.0.
    //
    // Through 0.49.0 the strict parser returned `Ok` for a body declaration it
    // could not parse, dropping it instead of reporting it — so a typo passed
    // `lint`, and a clean run was weaker evidence than it looked. 0.50.0's
    // verdict-parity work makes strict report what it used to drop.
    //
    // That change is also what surfaced the newly-failing models recorded in
    // docs/specs/03-verdict-parity-fallout.adoc: the same masking hid real
    // grammar gaps, not just these typos.
    for (label, src) in [
        (
            "missing semicolon",
            "package P { part def A { attribute x : String } }",
        ),
        ("misspelled keyword", "package P { attribut def X { } }"),
        (
            "missing colon",
            "package P { part def A { attribute x String; } }",
        ),
        (
            "unterminated string",
            "package P { part def A { attribute x : String = \"oops; } }",
        ),
        ("unknown keyword", "package P { frobnicate def Q { } }"),
        ("pluralized keyword", "package P { parts def A { } }"),
    ] {
        for mode in [
            sysml_v2_cli::lint::LintMode::Strict,
            sysml_v2_cli::lint::LintMode::Edit,
        ] {
            let (ok, errors) = sysml_v2_cli::lint::lint_source_mode(src, mode);
            assert!(!ok, "{mode:?} should reject {label}");
            assert!(!errors.is_empty(), "{mode:?} should explain {label}");
        }
    }

    // Criterion 4 is still unmet: neither mode rejects this. Recorded so the gap
    // is not mistaken for coverage.
    let garbage = "package P { part def A { %%% garbage %%% } }";
    for mode in [
        sysml_v2_cli::lint::LintMode::Strict,
        sysml_v2_cli::lint::LintMode::Edit,
    ] {
        let (ok, _) = sysml_v2_cli::lint::lint_source_mode(garbage, mode);
        assert!(ok, "{mode:?} is still expected to miss `%%% garbage %%%`");
    }
}

#[test]
fn both_modes_reject_uncovered_grammar() {
    // Constructs that are legal SysML v2 but that sysml-v2-parser 0.50.0 does not
    // implement. Both entry points reject them, consistently — these are grammar
    // gaps, not a mode disagreement.
    //
    // Every case below is corroborated by OMG-authored SysML v2 (the domain and
    // systems libraries, the PTC 2025 SimpleVehicleModel, or the training
    // examples) *in the same body kind*, and each is paired with an accepted
    // control in docs/specs/03-verdict-parity-fallout.adoc. That corroboration
    // matters: five earlier entries in this list turned out to be defects in our
    // own models rather than parser gaps, and were withdrawn.
    //
    // Do not "fix" a model to satisfy this test. When a release closes one of
    // these, this test fails — that is the signal to update spec 03.
    for (label, src) in [
        (
            // OMG: Parts.sysml:51, 3e-Function-based Behavior-item.sysml:28
            "part usage in an action-definition body",
            r#"
            package Foo {
                part def A { attribute x : String; }
                action def Run { part p : A; }
            }
            "#,
        ),
        (
            // Seven other `def` kinds are accepted in this exact position.
            "action def nested in a part-definition body",
            r#"
            package Foo {
                part def Outer {
                    action def Inner { }
                }
            }
            "#,
        ),
        (
            // OMG: 29 interface usage members inside part bodies.
            "interface usage as a part-definition body member",
            r#"
            package Foo {
                port def P;
                interface def I { end p1 : P; end p2 : P; }
                part def A { port p : P; }
                part def B {
                    part x : A;
                    part y : A;
                    interface xy : I;
                }
            }
            "#,
        ),
        (
            // OMG: ptc-25-04-31.sysml `exhibit state vehicleStates parallel {`
            "exhibit state with the parallel modifier",
            r#"
            package Foo {
                part def A {
                    exhibit state s parallel {
                        state on;
                        state off;
                    }
                }
            }
            "#,
        ),
        (
            // OMG: StructuredControlTest.sysml:32
            "typed loop variable in a for loop",
            r#"
            package Foo {
                action def Run {
                    for n : ScalarValues::Integer in (1, 2, 3) { action inner; }
                }
            }
            "#,
        ),
    ] {
        for mode in [
            sysml_v2_cli::lint::LintMode::Strict,
            sysml_v2_cli::lint::LintMode::Edit,
        ] {
            let (ok, errors) = sysml_v2_cli::lint::lint_source_mode(src, mode);
            assert!(
                !ok,
                "{mode:?} unexpectedly accepts {label} — the upstream gap may be \
                 closed; see the note above"
            );
            assert!(!errors.is_empty());
        }
    }
}

#[test]
fn accepts_the_valid_forms_of_constructs_we_once_misreported() {
    // Five entries were withdrawn from the gap list above after review: our models
    // were wrong, not the parser. These assert the *correct* SysML v2 spelling of
    // each, so the corrected models cannot silently regress and the withdrawals
    // cannot quietly come back.
    for (label, src) in [
        (
            "verification def, not `test case def`",
            "package P { part def W; requirement def R; \
             verification def V { subject s : W; verify r : R; } }",
        ),
        (
            "sequence constructor, not a bare comma list",
            "package P { part def G; part def F { part rs : G[*]; } \
             part def B :> F { part p : G; part t : G; part :>> rs = (p, t); } }",
        ),
        (
            "for, not foreach",
            "package P { action def R { for x in xs { action i; } } }",
        ),
        (
            "allocate with plain endpoints, no kind keyword",
            "package P { part def J; action def S; \
             part def T { part pr : J; action st : S; allocate st to pr; } }",
        ),
        (
            "one then per succession statement",
            "package P { action def L { action a; action b; action c; \
             first a then b; then c; } }",
        ),
    ] {
        for mode in [
            sysml_v2_cli::lint::LintMode::Strict,
            sysml_v2_cli::lint::LintMode::Edit,
        ] {
            let (ok, errors) = sysml_v2_cli::lint::lint_source_mode(src, mode);
            assert!(
                ok,
                "{mode:?} should accept {label}, got {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }
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
