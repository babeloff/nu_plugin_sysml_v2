//! Cross-file `import` resolution against a namespace-to-file library index
//! — e.g. a local checkout of the OMG SysML v2 Systems Library, covering
//! `ISQ`/`SI`.
//!
//! Opt-in only (the CLI's `--resolve-imports`/`--lib-dir` flags, the
//! plugin's equivalent) — the default `lint`/`emit` behavior stays
//! syntax-only, so this can never regress into the kind of false
//! "undefined reference" error that got `syster` replaced (see
//! `docs/explanation/why-sysml-v2-parser.adoc`).
//!
//! Scope: indexing and resolution only cover `package`, `library package`,
//! `item def`, `part def`, `enum def`, and `attribute def` declarations —
//! enough for ISQ/SI-style quantity/unit libraries, but not every SysML
//! construct (see `docs/explanation/architecture.adoc`). A reference whose
//! base package was never imported is out of scope for this pass (that's a
//! missing-import concern, not a missing-symbol one).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::lower::{
    collect_import_targets, collect_package_names, file_symbols_from_text, HirSymbol,
    PackageResolver, RelationshipKind,
};

/// Maps a fully-qualified SysML package name to the file that declares it.
#[derive(Debug, Default, Clone)]
pub struct LibraryIndex {
    package_files: HashMap<String, PathBuf>,
}

impl LibraryIndex {
    /// Recursively scan every directory in `lib_dirs` for `*.sysml` files,
    /// indexing every `package`/`library package` name each one declares
    /// (top-level and nested). Later directories don't override an already
    /// -indexed name from an earlier one.
    pub fn build(lib_dirs: &[PathBuf]) -> std::io::Result<Self> {
        let mut package_files = HashMap::new();
        for dir in lib_dirs {
            index_dir(dir, &mut package_files)?;
        }
        Ok(Self { package_files })
    }

    pub fn file_for_package(&self, qualified_package: &str) -> Option<&Path> {
        self.package_files.get(qualified_package).map(PathBuf::as_path)
    }

    pub fn is_empty(&self) -> bool {
        self.package_files.is_empty()
    }

    pub fn len(&self) -> usize {
        self.package_files.len()
    }
}

impl PackageResolver for LibraryIndex {
    fn resolve(&self, qualified_package: &str) -> Option<PathBuf> {
        self.file_for_package(qualified_package)
            .map(Path::to_path_buf)
    }
}

fn index_dir(dir: &Path, out: &mut HashMap<String, PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            index_dir(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("sysml") {
            if let Ok(src) = std::fs::read_to_string(&path) {
                for name in collect_package_names(&src) {
                    out.entry(name).or_insert_with(|| path.clone());
                }
            }
        }
    }
    Ok(())
}

/// One `import` statement that couldn't be resolved against the library
/// index — its target package isn't declared by any indexed file.
#[derive(Debug, Clone)]
pub struct UnresolvedImport {
    pub target: String,
    pub line: u32,
    pub column: usize,
}

/// One qualified type reference (e.g. `ISQ::MassValue`) whose base package
/// *was* imported and resolved, but the specific symbol wasn't found among
/// that package's (or its transitive imports') declared names.
#[derive(Debug, Clone)]
pub struct UnresolvedReference {
    /// Qualified name of the symbol making the reference.
    pub symbol: String,
    pub target: String,
    pub line: u32,
    pub column: usize,
}

/// Result of resolving one file's `import` statements against a
/// [`LibraryIndex`].
#[derive(Debug, Default)]
pub struct ResolvedImports {
    /// Symbols pulled in transitively from every successfully resolved import.
    pub symbols: Vec<HirSymbol>,
    pub unresolved_imports: Vec<UnresolvedImport>,
    pub unresolved_references: Vec<UnresolvedReference>,
}

/// Resolve every `import` in `source` against `index`, then cross-check
/// every qualified type reference in `source` (whose base package was
/// actually imported) against the combined own+imported symbol table.
pub fn resolve_imports(source: &str, index: &LibraryIndex) -> ResolvedImports {
    let mut result = ResolvedImports::default();
    let mut visited = HashSet::new();

    for imp in collect_import_targets(source) {
        let base = base_package(&imp.target).to_owned();
        if !resolve_package(&base, index, &mut visited, &mut result.symbols) {
            result.unresolved_imports.push(UnresolvedImport {
                target: imp.target,
                line: imp.line,
                column: imp.column,
            });
        }
    }

    let own_symbols = file_symbols_from_text(source);
    let known: HashSet<&str> = own_symbols
        .iter()
        .chain(result.symbols.iter())
        .map(|s| s.qualified_name.as_str())
        .collect();

    for sym in &own_symbols {
        for rel in &sym.relationships {
            if rel.kind != RelationshipKind::TypedBy {
                continue;
            }
            let target = rel.target.as_str();
            if target.is_empty() || !target.contains("::") {
                continue;
            }
            let base = base_package(target);
            if visited.contains(base) && !known.contains(target) {
                result.unresolved_references.push(UnresolvedReference {
                    symbol: sym.qualified_name.clone(),
                    target: target.to_owned(),
                    line: sym.span.0,
                    column: sym.span.1,
                });
            }
        }
    }

    result
}

/// Resolve one package (and, transitively, whatever it itself imports)
/// against `index`, extending `out` with its declared symbols. Returns
/// `false` if `qualified_package` isn't in the index. `visited` both
/// memoizes already-resolved packages and guards against import cycles
/// (e.g. two library files importing each other).
fn resolve_package(
    qualified_package: &str,
    index: &LibraryIndex,
    visited: &mut HashSet<String>,
    out: &mut Vec<HirSymbol>,
) -> bool {
    if visited.contains(qualified_package) {
        return true;
    }
    let Some(path) = index.file_for_package(qualified_package) else {
        return false;
    };
    visited.insert(qualified_package.to_owned());

    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };

    for imp in collect_import_targets(&src) {
        let base = base_package(&imp.target).to_owned();
        resolve_package(&base, index, visited, out);
    }

    out.extend(file_symbols_from_text(&src));
    true
}

fn base_package(target: &str) -> &str {
    target.split("::").next().unwrap_or(target)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn resolves_import_and_reference() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "isq.sysml",
            r#"
            standard library package ISQ {
                attribute def MassValue :> ScalarQuantityValue;
            }
            "#,
        );
        let index = LibraryIndex::build(&[dir.path().to_path_buf()]).unwrap();
        assert!(index.file_for_package("ISQ").is_some());

        let source = r#"
            package Foo {
                private import ISQ::*;
                attribute def Weighing {
                    attribute mass : ISQ::MassValue;
                }
            }
            "#;
        let resolved = resolve_imports(source, &index);
        assert!(resolved.unresolved_imports.is_empty());
        assert!(resolved.unresolved_references.is_empty());
        assert!(resolved
            .symbols
            .iter()
            .any(|s| s.qualified_name == "ISQ::MassValue"));
    }

    #[test]
    fn reports_unresolved_import() {
        let index = LibraryIndex::default();
        let source = r#"
            package Foo {
                private import ISQ::*;
            }
            "#;
        let resolved = resolve_imports(source, &index);
        assert_eq!(resolved.unresolved_imports.len(), 1);
        assert_eq!(resolved.unresolved_imports[0].target, "ISQ::*");
    }

    #[test]
    fn reports_unresolved_reference_within_resolved_import() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "isq.sysml",
            r#"
            standard library package ISQ {
                attribute def MassValue :> ScalarQuantityValue;
            }
            "#,
        );
        let index = LibraryIndex::build(&[dir.path().to_path_buf()]).unwrap();

        let source = r#"
            package Foo {
                private import ISQ::*;
                attribute def Weighing {
                    attribute mass : ISQ::NoSuchQuantity;
                }
            }
            "#;
        let resolved = resolve_imports(source, &index);
        assert!(resolved.unresolved_imports.is_empty());
        assert_eq!(resolved.unresolved_references.len(), 1);
        assert_eq!(resolved.unresolved_references[0].target, "ISQ::NoSuchQuantity");
    }

    #[test]
    fn resolves_transitively_imported_package() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "si.sysml",
            r#"
            standard library package SI {
                attribute def kg;
            }
            "#,
        );
        write(
            dir.path(),
            "isq.sysml",
            r#"
            standard library package ISQ {
                private import SI::*;
                attribute def MassValue :> SI::kg;
            }
            "#,
        );
        let index = LibraryIndex::build(&[dir.path().to_path_buf()]).unwrap();

        let source = r#"
            package Foo {
                private import ISQ::*;
                attribute def Weighing {
                    attribute mass : SI::kg;
                }
            }
            "#;
        let resolved = resolve_imports(source, &index);
        assert!(resolved.unresolved_imports.is_empty());
        assert!(
            resolved.unresolved_references.is_empty(),
            "{:?}",
            resolved.unresolved_references
        );
    }
}
