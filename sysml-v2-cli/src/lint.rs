//! `sysml-v2-cli lint` — SysML v2 syntax validation.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Serialize;
use sysml_v2_parser::{parse, parse_for_editor, ParseError};

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

/// Which parser entry point decides whether a file is valid.
///
/// Since `sysml-v2-parser` 0.50.0 the two agree on the verdict for every file
/// measured, so this selects how much detail you get rather than which answer to
/// believe. Before 0.50.0 they diverged in both directions — the strict path
/// silently dropped declarations it could not parse, and the recovery path
/// rejected constructs it had not implemented; see
/// `docs/specs/02-parser-inconsistency.adoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LintMode {
    /// Verdict from the strict parser, which stops at the first error. The
    /// default, and the right choice for validating a finished model or gating
    /// CI.
    #[default]
    Strict,
    /// Verdict from the error-recovery parser, as an editor or LSP would see it:
    /// every recoverable problem is reported rather than just the first. Useful
    /// while writing a file, or when fixing a model with several errors in it.
    Edit,
}

/// Lint a single in-memory SysML v2 source string in [`LintMode::Strict`].
///
/// Returns `(ok, errors)` — `ok` is `true` when the source parses without
/// syntax errors. This is the reusable core behind both the CLI's per-file
/// loop and the `nu_plugin_sysml_v2` `lint sysml` plugin command.
pub fn lint_source(source: &str) -> (bool, Vec<ErrorReport>) {
    lint_source_mode(source, LintMode::Strict)
}

/// Lint a single in-memory SysML v2 source string in an explicit mode.
///
/// In `Strict`, the recovery path is still consulted — but only once the strict
/// parse has already failed, because it enumerates every error in the file
/// instead of just the first, which is what makes the diagnostics useful.
pub fn lint_source_mode(source: &str, mode: LintMode) -> (bool, Vec<ErrorReport>) {
    match mode {
        LintMode::Edit => {
            let result = parse_for_editor(source);
            let ok = result.is_ok();
            let errors = result.errors.iter().map(ErrorReport::from).collect();
            (ok, errors)
        }
        LintMode::Strict => match parse(source) {
            Ok(_) => (true, Vec::new()),
            Err(first) => {
                let recovered = parse_for_editor(source);
                let mut errors: Vec<ErrorReport> =
                    recovered.errors.iter().map(ErrorReport::from).collect();
                // Recovery can disagree about *where* the failure is and, in
                // principle, find nothing at all; never report a failure with no
                // explanation attached.
                if errors.is_empty() {
                    errors.push(ErrorReport::from(&first));
                }
                (false, errors)
            }
        },
    }
}

/// Like [`lint_source`], but also resolves `source`'s `import` statements
/// against `index` and flags unresolved imports/references — see
/// `crate::resolve`. Opt-in (CLI `--resolve-imports`/`--lib-dir`, plugin
/// equivalent): plain [`lint_source`] never changes behavior.
pub fn lint_source_with_imports(
    source: &str,
    index: &crate::resolve::LibraryIndex,
) -> (bool, Vec<ErrorReport>) {
    lint_source_with_imports_mode(source, index, LintMode::Strict)
}

/// Like [`lint_source_with_imports`], with an explicit [`LintMode`] for the
/// syntax half of the check. Import resolution is unaffected by the mode.
pub fn lint_source_with_imports_mode(
    source: &str,
    index: &crate::resolve::LibraryIndex,
    mode: LintMode,
) -> (bool, Vec<ErrorReport>) {
    let (mut ok, mut errors) = lint_source_mode(source, mode);
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

pub fn run(files: Vec<PathBuf>, json: bool, mode: LintMode) -> Result<ExitCode> {
    run_impl(files, json, None, mode)
}

/// Like [`run`], but resolves each file's `import` statements against a
/// [`crate::resolve::LibraryIndex`] built from `lib_dirs`.
pub fn run_with_imports(
    files: Vec<PathBuf>,
    json: bool,
    lib_dirs: &[PathBuf],
    mode: LintMode,
) -> Result<ExitCode> {
    let index = crate::resolve::LibraryIndex::build(lib_dirs)
        .context("failed to scan --lib-dir directories")?;
    run_impl(files, json, Some(&index), mode)
}

fn run_impl(
    files: Vec<PathBuf>,
    json: bool,
    index: Option<&crate::resolve::LibraryIndex>,
    mode: LintMode,
) -> Result<ExitCode> {
    let mut reports = Vec::with_capacity(files.len());
    for file in &files {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let (ok, errors) = match index {
            Some(index) => lint_source_with_imports_mode(&source, index, mode),
            None => lint_source_mode(&source, mode),
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
