//! `sysml-v2-cli lint` — SysML v2 syntax validation.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Serialize;
use sysml_v2_parser::{parse_for_editor, ParseError};

// `sysml_v2_parser::ParseError` does not itself derive `Serialize` (only the
// `DiagnosticSeverity`/`DiagnosticCategory` enums nested inside it do), so we
// mirror the fields we care about into our own DTO for JSON output.
#[derive(Serialize, Clone)]
pub struct ErrorReport {
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<usize>,
    pub severity: Option<String>,
    pub category: Option<String>,
    pub code: Option<String>,
    pub expected: Option<String>,
    pub found: Option<String>,
    pub suggestion: Option<String>,
}

impl From<&ParseError> for ErrorReport {
    fn from(e: &ParseError) -> Self {
        ErrorReport {
            message: e.message.clone(),
            line: e.line,
            column: e.column,
            severity: e.severity.map(|s| format!("{s:?}")),
            category: e.category.map(|c| format!("{c:?}")),
            code: e.code.clone(),
            expected: e.expected.clone(),
            found: e.found.clone(),
            suggestion: e.suggestion.clone(),
        }
    }
}

#[derive(Serialize)]
struct FileReport {
    file: PathBuf,
    ok: bool,
    errors: Vec<ErrorReport>,
}

/// Lint a single in-memory SysML v2 source string.
///
/// Returns `(ok, errors)` — `ok` is `true` when the source parses without
/// syntax errors. This is the reusable core behind both the CLI's per-file
/// loop and the `nu_plugin_sysml_v2` `lint sysml` plugin command.
pub fn lint_source(source: &str) -> (bool, Vec<ErrorReport>) {
    let result = parse_for_editor(source);
    let ok = result.is_ok();
    let errors = result.errors.iter().map(ErrorReport::from).collect();
    (ok, errors)
}

/// Like [`lint_source`], but also resolves `source`'s `import` statements
/// against `index` and flags unresolved imports/references — see
/// `crate::resolve`. Opt-in (CLI `--resolve-imports`/`--lib-dir`, plugin
/// equivalent): plain [`lint_source`] never changes behavior.
pub fn lint_source_with_imports(
    source: &str,
    index: &crate::resolve::LibraryIndex,
) -> (bool, Vec<ErrorReport>) {
    let (mut ok, mut errors) = lint_source(source);
    let resolved = crate::resolve::resolve_imports(source, index);

    for u in &resolved.unresolved_imports {
        ok = false;
        errors.push(ErrorReport {
            message: format!("unresolved import: {}", u.target),
            line: Some(u.line),
            column: Some(u.column),
            severity: None,
            category: Some("UnresolvedSymbol".to_owned()),
            code: Some("unresolved_import".to_owned()),
            expected: None,
            found: None,
            suggestion: None,
        });
    }

    for u in &resolved.unresolved_references {
        ok = false;
        errors.push(ErrorReport {
            message: format!("unresolved reference to {} (from {})", u.target, u.symbol),
            line: Some(u.line),
            column: Some(u.column),
            severity: None,
            category: Some("UnresolvedSymbol".to_owned()),
            code: Some("unresolved_reference".to_owned()),
            expected: None,
            found: None,
            suggestion: None,
        });
    }

    (ok, errors)
}

pub fn run(files: Vec<PathBuf>, json: bool) -> Result<ExitCode> {
    run_impl(files, json, None)
}

/// Like [`run`], but resolves each file's `import` statements against a
/// [`crate::resolve::LibraryIndex`] built from `lib_dirs`.
pub fn run_with_imports(files: Vec<PathBuf>, json: bool, lib_dirs: &[PathBuf]) -> Result<ExitCode> {
    let index = crate::resolve::LibraryIndex::build(lib_dirs)
        .context("failed to scan --lib-dir directories")?;
    run_impl(files, json, Some(&index))
}

fn run_impl(
    files: Vec<PathBuf>,
    json: bool,
    index: Option<&crate::resolve::LibraryIndex>,
) -> Result<ExitCode> {
    let mut reports = Vec::with_capacity(files.len());
    for file in &files {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let (ok, errors) = match index {
            Some(index) => lint_source_with_imports(&source, index),
            None => lint_source(&source),
        };
        reports.push(FileReport {
            file: file.clone(),
            ok,
            errors,
        });
    }

    let any_errors = reports.iter().any(|r| !r.ok);

    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        print_text_report(&reports);
    }

    Ok(if any_errors {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn print_text_report(reports: &[FileReport]) {
    let mut total_errors = 0usize;
    for report in reports {
        if report.ok {
            println!("{}: ok", report.file.display());
            continue;
        }
        for err in &report.errors {
            total_errors += 1;
            let location = match (err.line, err.column) {
                (Some(line), Some(col)) => format!(":{line}:{col}"),
                _ => String::new(),
            };
            println!("{}{}: {}", report.file.display(), location, err.message);
        }
    }

    let file_word = if reports.len() == 1 { "file" } else { "files" };
    if total_errors > 0 {
        println!(
            "\u{2717} Analyzed {} {}: {} error(s)",
            reports.len(),
            file_word,
            total_errors
        );
    } else {
        println!(
            "\u{2713} Analyzed {} {}: no errors",
            reports.len(),
            file_word
        );
    }
}
