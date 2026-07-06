//! `sysml-v2-cli lint` — SysML v2 syntax validation.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Serialize;
use sysml_v2_parser::{parse_for_editor, ParseError};

// `sysml_v2_parser::ParseError` does not itself derive `Serialize` (only the
// `DiagnosticSeverity`/`DiagnosticCategory` enums nested inside it do), so we
// mirror the fields we care about into our own DTO for JSON output.
#[derive(Serialize)]
struct ErrorReport {
    message: String,
    line: Option<u32>,
    column: Option<usize>,
    severity: Option<String>,
    category: Option<String>,
    code: Option<String>,
    expected: Option<String>,
    found: Option<String>,
    suggestion: Option<String>,
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

pub fn run(files: Vec<PathBuf>, json: bool) -> Result<ExitCode> {
    let mut reports = Vec::with_capacity(files.len());
    for file in &files {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let result = parse_for_editor(&source);
        reports.push(FileReport {
            file: file.clone(),
            ok: result.is_ok(),
            errors: result.errors.iter().map(ErrorReport::from).collect(),
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
