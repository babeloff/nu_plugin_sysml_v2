//! SysML v2 AST → EmitModel lowering pass.
//!
//! Walks the `sysml-v2-parser` AST to build a flat [`HirSymbol`] list, then
//! lowers that list into an [`EmitModel`] ready for proto/XSD generation.
//!
//! Moved here from `channel-adapter/crate/sysml-emit` — the CoT message
//! schema domain (`derive_proto_package`, `proto_file_for_package`, etc.)
//! is channel-adapter-specific, but the AST-walking/lowering machinery
//! itself is generic SysML v2 processing, so it now lives alongside the
//! rest of the granule toolchain's SysML tooling in `sysml-v2-cli`.
//!
//! Also home to the AST-walking helpers ([`collect_package_names`],
//! [`collect_import_targets`]) that [`crate::resolve`] builds its
//! namespace-to-file library index and import resolution on top of.

use std::path::PathBuf;

use sysml_v2_parser::ast::{
    AttributeBody, AttributeBodyElement, EnumerationBody, Identification, Import, Node,
    PackageBody, PartDefBody,
};
use sysml_v2_parser::{parse_for_editor, PackageBodyElement, PartDefBodyElement, RootElement};

// ---------------------------------------------------------------------------
// Local HIR types  (replaces syster::hir::{HirSymbol, SymbolKind, …})
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    EnumerationDefinition,
    EnumerationVariant,
    ItemDefinition,
    /// `attribute def` — e.g. ISQ/SI quantity-kind and unit definitions.
    AttributeDefinition,
    AttributeUsage,
    ItemUsage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationshipKind {
    TypedBy,
}

#[derive(Debug, Clone)]
pub struct HirRelationship {
    pub kind: RelationshipKind,
    pub target: String,
}

/// Multiplicity extracted from a `[lo..hi]` string.
#[derive(Debug, Clone)]
pub struct Multiplicity {
    pub lower: Option<u32>,
    pub upper: Option<u32>,
}

/// One semantic symbol extracted from a SysML source file.
#[derive(Debug, Clone)]
pub struct HirSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub doc: Option<String>,
    pub multiplicity: Option<Multiplicity>,
    pub relationships: Vec<HirRelationship>,
    /// `(line, column)` of the declaration, 1-based — used to locate
    /// unresolved-reference diagnostics (see [`crate::resolve`]).
    pub span: (u32, usize),
}

// ---------------------------------------------------------------------------
// HIR builder  (replaces syster::hir::file_symbols_from_text)
// ---------------------------------------------------------------------------

/// Parse `source` and return a flat list of [`HirSymbol`]s in declaration order.
pub fn file_symbols_from_text(source: &str) -> Vec<HirSymbol> {
    let result = parse_for_editor(source);
    let mut symbols = Vec::new();
    for elem in &result.root.elements {
        match &elem.value {
            RootElement::Package(node) => {
                let pkg_name = ident_name(&node.identification);
                if let PackageBody::Brace { elements } = &node.body {
                    collect_pkg_symbols(elements, &pkg_name, &mut symbols);
                }
            }
            RootElement::LibraryPackage(node) => {
                let pkg_name = ident_name(&node.identification);
                if let PackageBody::Brace { elements } = &node.body {
                    collect_pkg_symbols(elements, &pkg_name, &mut symbols);
                }
            }
            _ => {}
        }
    }
    symbols
}

fn collect_pkg_symbols(
    elements: &[Node<PackageBodyElement>],
    parent_qname: &str,
    out: &mut Vec<HirSymbol>,
) {
    for elem in elements {
        let span = (elem.span.line, elem.span.column);
        match &elem.value {
            PackageBodyElement::EnumDef(node) => {
                let name = ident_name(&node.identification);
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                out.push(HirSymbol {
                    name: name.clone(),
                    qualified_name: qname.clone(),
                    kind: SymbolKind::EnumerationDefinition,
                    doc: None,
                    multiplicity: None,
                    relationships: vec![],
                    span,
                });
                collect_enum_variants(&node.body, &qname, span, out);
            }
            PackageBodyElement::ItemDef(node) => {
                let name = ident_name(&node.identification);
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                out.push(HirSymbol {
                    name: name.clone(),
                    qualified_name: qname.clone(),
                    kind: SymbolKind::ItemDefinition,
                    doc: None,
                    multiplicity: None,
                    relationships: vec![],
                    span,
                });
                if let AttributeBody::Brace { elements } = &node.body {
                    collect_attr_body_symbols(elements, &qname, out);
                }
            }
            PackageBodyElement::PartDef(node) => {
                let name = ident_name(&node.identification);
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                out.push(HirSymbol {
                    name: name.clone(),
                    qualified_name: qname.clone(),
                    kind: SymbolKind::ItemDefinition,
                    doc: None,
                    multiplicity: None,
                    relationships: vec![],
                    span,
                });
                if let PartDefBody::Brace { elements } = &node.body {
                    collect_partdef_body_symbols(elements, &qname, out);
                }
            }
            PackageBodyElement::AttributeDef(node) => {
                let name = node.name.trim_matches('\'').to_owned();
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                let relationships = node
                    .typing
                    .clone()
                    .map(|target| {
                        vec![HirRelationship {
                            kind: RelationshipKind::TypedBy,
                            target,
                        }]
                    })
                    .unwrap_or_default();
                out.push(HirSymbol {
                    name: name.clone(),
                    qualified_name: qname.clone(),
                    kind: SymbolKind::AttributeDefinition,
                    doc: None,
                    multiplicity: None,
                    relationships,
                    span,
                });
                if let AttributeBody::Brace { elements } = &node.body {
                    collect_attr_body_symbols(elements, &qname, out);
                }
            }
            PackageBodyElement::Package(node) => {
                let name = ident_name(&node.identification);
                let qname = qualify(parent_qname, &name);
                if let PackageBody::Brace { elements } = &node.body {
                    collect_pkg_symbols(elements, &qname, out);
                }
            }
            PackageBodyElement::LibraryPackage(node) => {
                let name = ident_name(&node.identification);
                let qname = qualify(parent_qname, &name);
                if let PackageBody::Brace { elements } = &node.body {
                    collect_pkg_symbols(elements, &qname, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_partdef_body_symbols(
    elements: &[Node<PartDefBodyElement>],
    parent_qname: &str,
    out: &mut Vec<HirSymbol>,
) {
    for elem in elements {
        let span = (elem.span.line, elem.span.column);
        match &elem.value {
            PartDefBodyElement::AttributeUsage(node) => {
                let name = node.name.trim_matches('\'').to_owned();
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                let typed_by = node.typing.clone().unwrap_or_default();
                out.push(HirSymbol {
                    name,
                    qualified_name: qname,
                    kind: SymbolKind::AttributeUsage,
                    doc: None,
                    multiplicity: None,
                    relationships: vec![HirRelationship {
                        kind: RelationshipKind::TypedBy,
                        target: typed_by,
                    }],
                    span,
                });
            }
            PartDefBodyElement::ItemUsage(node) => {
                let name = node.name.trim_matches('\'').to_owned();
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                let typed_by = node.type_name.clone().unwrap_or_default();
                let mult = node.multiplicity.as_deref().map(parse_multiplicity);
                out.push(HirSymbol {
                    name,
                    qualified_name: qname,
                    kind: SymbolKind::ItemUsage,
                    doc: None,
                    multiplicity: mult,
                    relationships: vec![HirRelationship {
                        kind: RelationshipKind::TypedBy,
                        target: typed_by,
                    }],
                    span,
                });
            }
            PartDefBodyElement::ItemDef(node) => {
                let name = ident_name(&node.identification);
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                out.push(HirSymbol {
                    name: name.clone(),
                    qualified_name: qname.clone(),
                    kind: SymbolKind::ItemDefinition,
                    doc: None,
                    multiplicity: None,
                    relationships: vec![],
                    span,
                });
                if let AttributeBody::Brace { elements } = &node.body {
                    collect_attr_body_symbols(elements, &qname, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_attr_body_symbols(
    elements: &[Node<AttributeBodyElement>],
    parent_qname: &str,
    out: &mut Vec<HirSymbol>,
) {
    for elem in elements {
        let span = (elem.span.line, elem.span.column);
        match &elem.value {
            AttributeBodyElement::AttributeUsage(node) => {
                let name = node.name.trim_matches('\'').to_owned();
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                let typed_by = node.typing.clone().unwrap_or_default();
                out.push(HirSymbol {
                    name,
                    qualified_name: qname,
                    kind: SymbolKind::AttributeUsage,
                    doc: None,
                    multiplicity: None,
                    relationships: vec![HirRelationship {
                        kind: RelationshipKind::TypedBy,
                        target: typed_by,
                    }],
                    span,
                });
            }
            // Nested `attribute def` — ISQ/SI define quantity-kind
            // hierarchies this way, e.g. `attribute def MassValue :>
            // ScalarQuantityValue { attribute def ... }`.
            AttributeBodyElement::AttributeDef(node) => {
                let name = node.name.trim_matches('\'').to_owned();
                if name.is_empty() {
                    continue;
                }
                let qname = qualify(parent_qname, &name);
                let relationships = node
                    .typing
                    .clone()
                    .map(|target| {
                        vec![HirRelationship {
                            kind: RelationshipKind::TypedBy,
                            target,
                        }]
                    })
                    .unwrap_or_default();
                out.push(HirSymbol {
                    name: name.clone(),
                    qualified_name: qname.clone(),
                    kind: SymbolKind::AttributeDefinition,
                    doc: None,
                    multiplicity: None,
                    relationships,
                    span,
                });
                if let AttributeBody::Brace { elements } = &node.body {
                    collect_attr_body_symbols(elements, &qname, out);
                }
            }
            _ => {}
        }
    }
}

/// Collect enum variant names from an `EnumerationBody`.
///
/// `EnumerationBody::Brace { values: Vec<String> }` — variants are plain strings.
fn collect_enum_variants(
    body: &EnumerationBody,
    parent_qname: &str,
    span: (u32, usize),
    out: &mut Vec<HirSymbol>,
) {
    if let EnumerationBody::Brace { values } = body {
        for raw in values {
            let name = raw.trim_matches('\'').to_owned();
            if name.is_empty() {
                continue;
            }
            out.push(HirSymbol {
                name: name.clone(),
                qualified_name: qualify(parent_qname, &name),
                kind: SymbolKind::EnumerationVariant,
                doc: None,
                multiplicity: None,
                relationships: vec![],
                span,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Package-name and import indexing (for crate::resolve)
// ---------------------------------------------------------------------------

/// Every `package`/`library package` name declared anywhere in `source`
/// (top-level and nested, fully qualified), so a directory of many small
/// library files — e.g. an ISQ/SI checkout — can be indexed by every
/// namespace it defines. See [`crate::resolve::LibraryIndex`].
pub fn collect_package_names(source: &str) -> Vec<String> {
    let result = parse_for_editor(source);
    let mut names = Vec::new();
    for elem in &result.root.elements {
        match &elem.value {
            RootElement::Package(node) => {
                collect_package_names_rec(&ident_name(&node.identification), &node.body, "", &mut names);
            }
            RootElement::LibraryPackage(node) => {
                collect_package_names_rec(&ident_name(&node.identification), &node.body, "", &mut names);
            }
            _ => {}
        }
    }
    names
}

fn collect_package_names_rec(name: &str, body: &PackageBody, parent: &str, out: &mut Vec<String>) {
    if name.is_empty() {
        return;
    }
    let qname = qualify(parent, name);
    out.push(qname.clone());
    if let PackageBody::Brace { elements } = body {
        for elem in elements {
            match &elem.value {
                PackageBodyElement::Package(node) => {
                    collect_package_names_rec(&ident_name(&node.identification), &node.body, &qname, out);
                }
                PackageBodyElement::LibraryPackage(node) => {
                    collect_package_names_rec(&ident_name(&node.identification), &node.body, &qname, out);
                }
                _ => {}
            }
        }
    }
}

/// One `import` statement's target, with the location of its qualified
/// name (excluding any `::*`/`::**` suffix).
#[derive(Debug, Clone)]
pub struct ImportTarget {
    pub target: String,
    pub line: u32,
    pub column: usize,
}

/// Every `import` statement anywhere in `source` (top-level or nested in
/// any package/library package body). See [`crate::resolve::resolve_imports`].
pub fn collect_import_targets(source: &str) -> Vec<ImportTarget> {
    let result = parse_for_editor(source);
    let mut out = Vec::new();
    for elem in &result.root.elements {
        match &elem.value {
            RootElement::Import(node) => out.push(import_target(node)),
            RootElement::Package(node) => collect_import_targets_rec(&node.body, &mut out),
            RootElement::LibraryPackage(node) => collect_import_targets_rec(&node.body, &mut out),
            _ => {}
        }
    }
    out
}

fn collect_import_targets_rec(body: &PackageBody, out: &mut Vec<ImportTarget>) {
    if let PackageBody::Brace { elements } = body {
        for elem in elements {
            match &elem.value {
                PackageBodyElement::Import(node) => out.push(import_target(node)),
                PackageBodyElement::Package(node) => collect_import_targets_rec(&node.body, out),
                PackageBodyElement::LibraryPackage(node) => collect_import_targets_rec(&node.body, out),
                _ => {}
            }
        }
    }
}

fn import_target(node: &Node<Import>) -> ImportTarget {
    ImportTarget {
        target: node.target.clone(),
        line: node.target_span.line,
        column: node.target_span.column,
    }
}

// ---------------------------------------------------------------------------
// Multiplicity parsing
// ---------------------------------------------------------------------------

/// Parse a SysML multiplicity string like `"[0..*]"`, `"[1]"`, `"[0..1]"`.
fn parse_multiplicity(raw: &str) -> Multiplicity {
    let inner = raw
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    if let Some((lo_s, hi_s)) = inner.split_once("..") {
        let lower = lo_s.trim().parse().ok();
        let upper = hi_s.trim().parse().ok(); // "*" parses as None → unbounded
        Multiplicity { lower, upper }
    } else {
        let n: Option<u32> = inner.trim().parse().ok();
        Multiplicity { lower: n, upper: n }
    }
}

// ---------------------------------------------------------------------------
// AST helpers
// ---------------------------------------------------------------------------

fn ident_name(id: &Identification) -> String {
    id.name
        .as_deref()
        .unwrap_or_default()
        .trim_matches('\'')
        .to_owned()
}

fn qualify(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}::{name}")
    }
}

// ---------------------------------------------------------------------------
// Emit model
// ---------------------------------------------------------------------------

/// Top-level model for one SysML package / file.
#[derive(Debug, Default)]
pub struct EmitModel {
    /// SysML package name (e.g. `CotDetailMsg`).
    pub sysml_name: String,
    /// Proto3 package name (e.g. `rsdk.cot.detail`).
    pub proto_package: String,
    /// XSD target namespace (e.g. `urn:rsdk:cot:detail`).
    pub xsd_namespace: String,
    /// Filenames to import in the generated `.proto` (e.g. `"cotdetail.proto"`).
    pub proto_imports: Vec<String>,
    /// `(namespace, schemaLocation)` pairs for XSD `xs:import`.
    pub xsd_imports: Vec<(String, String)>,
    /// Top-level definitions (messages and enums) in declaration order.
    pub definitions: Vec<EmitDef>,
}

#[derive(Debug)]
pub enum EmitDef {
    Enum(EmitEnum),
    Message(EmitMessage),
}

#[derive(Debug)]
pub struct EmitEnum {
    pub name: String,
    pub doc: Option<String>,
    /// Variant names in declaration order.
    pub variants: Vec<String>,
}

#[derive(Debug)]
pub struct EmitMessage {
    pub name: String,
    pub doc: Option<String>,
    pub fields: Vec<EmitField>,
    /// Nested enum / message definitions (e.g. proto nested types).
    pub nested: Vec<EmitDef>,
}

#[derive(Debug)]
pub struct EmitField {
    pub name: String,
    /// Resolved type name (proto scalar or message name).
    pub ty: String,
    pub mult: EmitMult,
    /// `true` when `ty` is a proto scalar (string, int64, …).
    pub is_scalar: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitMult {
    /// `[1]` or no annotation → proto singular field.
    Required,
    /// `[0..1]` → proto `optional` field.
    Optional,
    /// `[0..*]` or `[1..*]` → proto `repeated` field.
    Repeated,
}

// ---------------------------------------------------------------------------
// Cross-package import resolution (used by crate::resolve::LibraryIndex)
// ---------------------------------------------------------------------------

/// Looks up the file that declares a given fully-qualified SysML package
/// name, so `lower_file_with_resolver` can derive a real cross-package
/// `.proto`/`.xsd` import instead of guessing from the hardcoded granule
/// package table below. Implemented by [`crate::resolve::LibraryIndex`].
pub trait PackageResolver {
    fn resolve(&self, qualified_package: &str) -> Option<PathBuf>;
}

// ---------------------------------------------------------------------------
// Package-name mappings
// ---------------------------------------------------------------------------

fn derive_proto_package(pkg: &str) -> &str {
    match pkg {
        "CotEventMsg" => "rsdk.cot.proto",
        "CotDetailMsg" => "rsdk.cot.detail",
        "CotDetailAtakMsg" => "rsdk.cot.detail.atak",
        "CotDetailUsMilMsg" => "rsdk.cot.detail.usmil",
        "CotDetailFtsMsg" => "rsdk.cot.detail.fts",
        "CotDetailAtakCivMsg" => "rsdk.cot.detail.atakciv",
        "CotGuessAtakMsg" => "rsdk.cot.guess.atak",
        "CotGuessFtsMsg" => "rsdk.cot.guess.fts",
        "CotGuessMisbMsg" => "rsdk.cot.guess.misb",
        other => other,
    }
}

fn derive_xsd_namespace(proto_pkg: &str) -> String {
    format!("urn:{}", proto_pkg.replace('.', ":"))
}

/// Proto file name that defines a given top-level SysML package name.
fn proto_file_for_package(pkg: &str) -> Option<&str> {
    match pkg {
        "CotEventMsg" => Some("cotevent.proto"),
        "CotDetailMsg" => Some("cotdetail.proto"),
        "CotDetailAtakMsg" => Some("cotdetail-atak.proto"),
        "CotDetailUsMilMsg" => Some("cotdetail-usmil.proto"),
        "CotDetailFtsMsg" => Some("cotdetail-fts.proto"),
        "CotDetailAtakCivMsg" => Some("cotdetail-atakciv.proto"),
        "CotGuessAtakMsg" => Some("cotguess-atak.proto"),
        "CotGuessFtsMsg" => Some("cotguess-fts.proto"),
        "CotGuessMisbMsg" => Some("cotguess-misb.proto"),
        _ => None,
    }
}

/// XSD file name that defines a given top-level SysML package name.
fn xsd_file_for_package(pkg: &str) -> Option<&str> {
    match pkg {
        "CotEventMsg" => Some("cotevent.xsd"),
        "CotDetailMsg" => Some("cotdetail.xsd"),
        "CotDetailAtakMsg" => Some("cotdetail-atak.xsd"),
        "CotDetailUsMilMsg" => Some("cotdetail-usmil.xsd"),
        "CotDetailFtsMsg" => Some("cotdetail-fts.xsd"),
        "CotDetailAtakCivMsg" => Some("cotdetail-atakciv.xsd"),
        "CotGuessAtakMsg" => Some("cotguess-atak.xsd"),
        "CotGuessFtsMsg" => Some("cotguess-fts.xsd"),
        "CotGuessMisbMsg" => Some("cotguess-misb.xsd"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// SysML → proto type mapping
// ---------------------------------------------------------------------------

/// Map SysML scalar type names to proto3 scalar type names.
/// Returns `(proto_type, is_scalar)`.
fn sysml_to_proto_type(raw: &str) -> (&str, bool) {
    match raw {
        "String" | "string" => ("string", true),
        "Integer" | "integer" => ("int64", true),
        "Real" | "real" => ("double", true),
        "Boolean" | "boolean" => ("bool", true),
        _ => (raw, false),
    }
}

// ---------------------------------------------------------------------------
// Multiplicity conversion
// ---------------------------------------------------------------------------

fn lower_mult(m: Option<&Multiplicity>) -> EmitMult {
    match m {
        None => EmitMult::Required,
        Some(m) => {
            let lo = m.lower.unwrap_or(0);
            let hi = m.upper;
            match (lo, hi) {
                (0, Some(1)) => EmitMult::Optional,
                (1, Some(1)) => EmitMult::Required,
                _ => EmitMult::Repeated,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Parse the first `package <Name>` or `package <Name> {` from SysML source.
pub fn parse_package_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("package ") {
            let name = rest.trim().trim_end_matches('{').trim();
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }
    }
    None
}

/// Lower a single SysML v2 source string into an [`EmitModel`].
///
/// `package_name` is the SysML package identifier (e.g. `"CotDetailMsg"`).
pub fn lower_file(source: &str, package_name: impl Into<String>) -> EmitModel {
    lower_file_with_resolver(source, package_name, None)
}

/// Like [`lower_file`], but cross-package `.proto`/`.xsd` imports are
/// derived from `resolver` (when it resolves a referenced package) instead
/// of the hardcoded granule package table, falling back to that table when
/// `resolver` is `None` or doesn't resolve a given package.
pub fn lower_file_with_resolver(
    source: &str,
    package_name: impl Into<String>,
    resolver: Option<&dyn PackageResolver>,
) -> EmitModel {
    let pkg = package_name.into();
    let proto_package = derive_proto_package(&pkg).to_owned();
    let xsd_namespace = derive_xsd_namespace(&proto_package);

    let symbols = file_symbols_from_text(source);

    let mut model = EmitModel {
        sysml_name: pkg,
        proto_package,
        xsd_namespace,
        ..Default::default()
    };

    collect_imports(&symbols, &mut model, resolver);

    for sym in &symbols {
        let depth = sym.qualified_name.chars().filter(|&c| c == ':').count();
        if depth != 2 {
            continue;
        }
        match sym.kind {
            SymbolKind::EnumerationDefinition => {
                let def = lower_enum(&symbols, sym);
                model.definitions.push(EmitDef::Enum(def));
            }
            SymbolKind::ItemDefinition => {
                let def = lower_message(&symbols, sym);
                model.definitions.push(EmitDef::Message(def));
            }
            _ => {}
        }
    }

    model
}

// ---------------------------------------------------------------------------
// Import collection
// ---------------------------------------------------------------------------

fn collect_imports(
    symbols: &[HirSymbol],
    model: &mut EmitModel,
    resolver: Option<&dyn PackageResolver>,
) {
    let mut seen_protos: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_xsd: std::collections::HashSet<String> = std::collections::HashSet::new();

    for sym in symbols {
        for rel in &sym.relationships {
            let raw = rel.target.as_str();
            let Some(sep) = raw.find("::") else { continue };
            let ref_pkg = &raw[..sep];
            if ref_pkg == model.sysml_name {
                continue;
            }

            if let Some(path) = resolver.and_then(|r| r.resolve(ref_pkg)) {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(ref_pkg)
                    .to_owned();
                let proto_file = format!("{stem}.proto");
                if seen_protos.insert(proto_file.clone()) {
                    model.proto_imports.push(proto_file);
                }
                let xsd_file = format!("{stem}.xsd");
                let ns = derive_xsd_namespace(ref_pkg);
                let key = format!("{ns}|{xsd_file}");
                if seen_xsd.insert(key) {
                    model.xsd_imports.push((ns, xsd_file));
                }
                continue;
            }

            // Fall back to the hardcoded granule message-package table
            // (no resolver given, or it didn't recognize this package).
            if let Some(proto_file) = proto_file_for_package(ref_pkg) {
                if seen_protos.insert(proto_file.to_owned()) {
                    model.proto_imports.push(proto_file.to_owned());
                }
            }
            if let Some(xsd_file) = xsd_file_for_package(ref_pkg) {
                let ns = derive_xsd_namespace(derive_proto_package(ref_pkg));
                let key = format!("{ns}|{xsd_file}");
                if seen_xsd.insert(key) {
                    model.xsd_imports.push((ns, xsd_file.to_owned()));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Enum lowering
// ---------------------------------------------------------------------------

fn lower_enum(symbols: &[HirSymbol], sym: &HirSymbol) -> EmitEnum {
    let variants = child_symbols(symbols, &sym.qualified_name)
        .into_iter()
        .filter(|s| s.kind == SymbolKind::EnumerationVariant)
        .map(|s| s.name.clone())
        .collect();
    EmitEnum {
        name: sym.name.clone(),
        doc: sym.doc.clone(),
        variants,
    }
}

// ---------------------------------------------------------------------------
// Message lowering (recursive for nested types)
// ---------------------------------------------------------------------------

fn lower_message(symbols: &[HirSymbol], sym: &HirSymbol) -> EmitMessage {
    let mut msg = EmitMessage {
        name: sym.name.clone(),
        doc: sym.doc.clone(),
        fields: Vec::new(),
        nested: Vec::new(),
    };

    let children = child_symbols(symbols, &sym.qualified_name);

    for child in &children {
        match child.kind {
            SymbolKind::EnumerationDefinition => {
                msg.nested.push(EmitDef::Enum(lower_enum(symbols, child)));
            }
            SymbolKind::ItemDefinition => {
                msg.nested
                    .push(EmitDef::Message(lower_message(symbols, child)));
            }
            SymbolKind::AttributeUsage => {
                let raw_ty = typed_by_raw(child);
                let (proto_ty, is_scalar) = sysml_to_proto_type(&raw_ty);
                msg.fields.push(EmitField {
                    name: to_snake_case(&child.name),
                    ty: proto_ty.to_owned(),
                    mult: lower_mult(child.multiplicity.as_ref()),
                    is_scalar,
                });
            }
            SymbolKind::ItemUsage => {
                let raw_ty = typed_by_raw(child);
                let short_ty = short_name(&raw_ty);
                msg.fields.push(EmitField {
                    name: to_snake_case(&child.name),
                    ty: short_ty,
                    mult: lower_mult(child.multiplicity.as_ref()),
                    is_scalar: false,
                });
            }
            _ => {}
        }
    }

    msg
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn child_symbols<'a>(symbols: &'a [HirSymbol], parent_qname: &str) -> Vec<&'a HirSymbol> {
    symbols
        .iter()
        .filter(|s| {
            s.qualified_name.starts_with(parent_qname)
                && s.qualified_name.len() > parent_qname.len()
                && {
                    let rest = &s.qualified_name[parent_qname.len()..];
                    rest.starts_with("::") && !rest[2..].contains("::")
                }
        })
        .collect()
}

fn typed_by_raw(sym: &HirSymbol) -> String {
    sym.relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::TypedBy)
        .map(|r| r.target.clone())
        .unwrap_or_else(|| "string".to_owned())
}

/// Strip a `Package::` qualifier, returning only the short name.
fn short_name(raw: &str) -> String {
    if let Some(sep) = raw.rfind("::") {
        raw[sep + 2..].to_owned()
    } else {
        raw.to_owned()
    }
}

/// Convert camelCase / PascalCase to snake_case for proto field names.
pub fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_upper = false;
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if i > 0 && (!prev_upper || next_lower) {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_upper = true;
        } else {
            out.push(c);
            prev_upper = false;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_basic() {
        assert_eq!(
            to_snake_case("platformHeadingAngle"),
            "platform_heading_angle"
        );
        assert_eq!(to_snake_case("uid"), "uid");
        assert_eq!(to_snake_case("xmlDetail"), "xml_detail");
        assert_eq!(to_snake_case("HAE"), "hae");
        assert_eq!(to_snake_case("sendTime"), "send_time");
    }

    #[test]
    fn attribute_def_at_package_level_is_indexed() {
        let src = r#"
            standard library package ISQ {
                attribute def MassValue :> ScalarQuantityValue;
                attribute def LengthValue :> ScalarQuantityValue;
            }
        "#;
        let symbols = file_symbols_from_text(src);
        assert!(symbols
            .iter()
            .any(|s| s.qualified_name == "ISQ::MassValue"
                && s.kind == SymbolKind::AttributeDefinition));
        assert!(symbols
            .iter()
            .any(|s| s.qualified_name == "ISQ::LengthValue"));
    }

    #[test]
    fn nested_attribute_def_is_indexed() {
        let src = r#"
            package SI {
                attribute def Units {
                    attribute def kg :> Units;
                }
            }
        "#;
        let symbols = file_symbols_from_text(src);
        assert!(symbols.iter().any(|s| s.qualified_name == "SI::Units::kg"));
    }

    #[test]
    fn collects_package_names_including_nested() {
        let src = r#"
            package Outer {
                package Inner {
                    attribute def X;
                }
            }
        "#;
        let names = collect_package_names(src);
        assert!(names.contains(&"Outer".to_owned()));
        assert!(names.contains(&"Outer::Inner".to_owned()));
    }

    #[test]
    fn collects_import_targets() {
        let src = r#"
            package Foo {
                private import ISQ::*;
                import SI::kg;
            }
        "#;
        let targets = collect_import_targets(src);
        let names: Vec<&str> = targets.iter().map(|t| t.target.as_str()).collect();
        assert!(names.contains(&"ISQ::*"));
        assert!(names.contains(&"SI::kg"));
    }
}
