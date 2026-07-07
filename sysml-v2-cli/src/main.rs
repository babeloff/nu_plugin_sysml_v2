//! sysml-v2-cli — SysML v2 tooling for the granule toolchain, built on
//! `sysml-v2-parser`.
//!
//! `lint` replaces the `syster` binary (syster-base/syster-cli), which
//! produced false "undefined reference" errors on valid files — it attempts
//! stdlib symbol resolution and its `ISQ`/`SI` model is incomplete — and
//! mis-parsed some identifiers that collide with grammar keywords in certain
//! positions. `sysml-v2-parser` is syntax-only: it never attempts import or
//! stdlib resolution (see its `DiagnosticCategory::UnresolvedSymbol`, which
//! the parser itself never emits — it exists only for callers to classify
//! their own semantic findings). That means `lint` cannot catch type errors,
//! but it also cannot produce that class of false positive.
//!
//! `--resolve-imports`/`--lib-dir` opts into an additional, separate check
//! (see `sysml_v2_cli::resolve`): resolving `import` statements against a
//! namespace-to-file library index (e.g. an ISQ/SI checkout) and flagging
//! unresolved imports/references. It's off by default so plain `lint`/`emit`
//! can never regress into the class of false positive `syster` produced.
//!
//! `emit` was moved here from `channel-adapter/crate/sysml-emit`: reads
//! SysML v2 message schemas and generates `.proto`/`.xsd` files. The
//! AST-walking/lowering machinery it depends on (`lower`, `emit_proto`,
//! `emit_xsd`) is generic SysML v2 processing, not specific to any one
//! granule package.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use sysml_v2_cli::lint;

#[derive(Parser)]
#[command(
    name = "sysml-v2-cli",
    about = "SysML v2 tooling for the granule toolchain"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate SysML v2 file syntax.
    Lint(LintArgs),
    /// Generate .proto and .xsd files from SysML v2 message schemas.
    Emit(EmitArgs),
}

#[derive(Args)]
struct LintArgs {
    /// SysML v2 file(s) to validate.
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Emit results as JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Resolve `import` statements against --lib-dir and flag unresolved
    /// imports/references (see sysml_v2_cli::resolve). Off by default.
    #[arg(long)]
    resolve_imports: bool,

    /// Directory to search for importable SysML files (repeatable). Only
    /// used with --resolve-imports.
    #[arg(long = "lib-dir")]
    lib_dirs: Vec<PathBuf>,
}

#[derive(Args)]
struct EmitArgs {
    /// SysML v2 file(s) to translate.
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Directory to write `<stem>.proto` for each input (default: stdout).
    #[arg(long)]
    proto_dir: Option<PathBuf>,

    /// Directory to write `<stem>.xsd` for each input (default: stdout).
    #[arg(long)]
    xsd_dir: Option<PathBuf>,

    /// Exact output path for the `.proto` file (single input only).
    #[arg(long)]
    proto_out: Option<PathBuf>,

    /// Exact output path for the `.xsd` file (single input only).
    #[arg(long)]
    xsd_out: Option<PathBuf>,

    /// Resolve cross-package `.proto`/`.xsd` imports against --lib-dir
    /// instead of the hardcoded granule package table. Off by default.
    #[arg(long)]
    resolve_imports: bool,

    /// Directory to search for importable SysML files (repeatable). Only
    /// used with --resolve-imports.
    #[arg(long = "lib-dir")]
    lib_dirs: Vec<PathBuf>,
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Lint(args) => {
            if args.resolve_imports {
                lint::run_with_imports(args.files, args.json, &args.lib_dirs)
            } else {
                lint::run(args.files, args.json)
            }
        }
        Command::Emit(args) => {
            emit::run(
                args.files,
                args.proto_dir,
                args.xsd_dir,
                args.proto_out,
                args.xsd_out,
                args.resolve_imports,
                args.lib_dirs,
            )?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

mod emit {
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    use sysml_v2_cli::{
        emit_proto::emit_proto,
        emit_xsd::emit_xsd,
        lower::{lower_file_with_resolver, parse_package_name, PackageResolver},
        resolve::LibraryIndex,
    };

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        files: Vec<PathBuf>,
        proto_dir: Option<PathBuf>,
        xsd_dir: Option<PathBuf>,
        proto_out: Option<PathBuf>,
        xsd_out: Option<PathBuf>,
        resolve_imports: bool,
        lib_dirs: Vec<PathBuf>,
    ) -> Result<()> {
        let index = if resolve_imports {
            Some(LibraryIndex::build(&lib_dirs).context("failed to scan --lib-dir directories")?)
        } else {
            None
        };
        let resolver: Option<&dyn PackageResolver> =
            index.as_ref().map(|i| i as &dyn PackageResolver);

        for path in &files {
            let src = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;

            // Derive package name from the SysML source (first package
            // declaration), falling back to the file stem.
            let pkg_name = parse_package_name(&src).unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("model")
                    .to_owned()
            });

            if let Some(index) = &index {
                let resolved = sysml_v2_cli::resolve::resolve_imports(&src, index);
                for u in &resolved.unresolved_imports {
                    eprintln!(
                        "warning: {}:{}:{}: unresolved import: {}",
                        path.display(),
                        u.line,
                        u.column,
                        u.target
                    );
                }
                for u in &resolved.unresolved_references {
                    eprintln!(
                        "warning: {}:{}:{}: unresolved reference: {} (from {})",
                        path.display(),
                        u.line,
                        u.column,
                        u.target,
                        u.symbol
                    );
                }
            }

            let model = lower_file_with_resolver(&src, &pkg_name, resolver);
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");

            // ── proto output ─────────────────────────────────────────────
            let proto_text = emit_proto(&model);
            let proto_dest = proto_out.clone().or_else(|| {
                proto_dir
                    .as_ref()
                    .map(|dir| dir.join(format!("{stem}.proto")))
            });
            match proto_dest {
                Some(dest) => {
                    write_if_changed(&dest, &proto_text)
                        .with_context(|| format!("writing {}", dest.display()))?;
                    eprintln!("proto: {}", dest.display());
                }
                None => {
                    println!("// ── {stem}.proto ────────────────────────────────────────────────");
                    print!("{proto_text}");
                }
            }

            // ── XSD output ───────────────────────────────────────────────
            let xsd_text = emit_xsd(&model);
            let xsd_dest = xsd_out
                .clone()
                .or_else(|| xsd_dir.as_ref().map(|dir| dir.join(format!("{stem}.xsd"))));
            match xsd_dest {
                Some(dest) => {
                    write_if_changed(&dest, &xsd_text)
                        .with_context(|| format!("writing {}", dest.display()))?;
                    eprintln!("xsd:   {}", dest.display());
                }
                None => {
                    println!("// ── {stem}.xsd ──────────────────────────────────────────────────");
                    print!("{xsd_text}");
                }
            }
        }

        Ok(())
    }

    /// Only write to disk when the content has changed (avoids spurious rebuilds).
    fn write_if_changed(path: &Path, content: &str) -> Result<()> {
        if let Ok(existing) = std::fs::read_to_string(path) {
            if existing == content {
                return Ok(());
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }
}
