//! EmitModel → proto3 text.
//!
//! Produces a single `.proto` file for one SysML package.  The output
//! format targets proto3 and mirrors the hand-written protos in `protos/`.

use crate::lower::{EmitDef, EmitEnum, EmitField, EmitMessage, EmitModel, EmitMult};

/// Render an [`EmitModel`] as proto3 source text.
pub fn emit_proto(model: &EmitModel) -> String {
    let mut out = String::new();

    // Header
    out.push_str("syntax = \"proto3\";\n\n");
    push_line(&mut out, &format!("package {};", model.proto_package));
    out.push('\n');
    out.push_str("option optimize_for = LITE_RUNTIME;\n");
    out.push('\n');

    // Imports
    for import in &model.proto_imports {
        push_line(&mut out, &format!("import \"{import}\";"));
    }
    if !model.proto_imports.is_empty() {
        out.push('\n');
    }

    // Definitions
    for def in &model.definitions {
        emit_def(&mut out, def, 0);
    }

    out
}

// ---------------------------------------------------------------------------
// Definition dispatch
// ---------------------------------------------------------------------------

fn emit_def(out: &mut String, def: &EmitDef, indent: usize) {
    match def {
        EmitDef::Enum(e) => emit_enum(out, e, indent),
        EmitDef::Message(m) => emit_message(out, m, indent),
    }
}

// ---------------------------------------------------------------------------
// Enum
// ---------------------------------------------------------------------------

fn emit_enum(out: &mut String, e: &EmitEnum, indent: usize) {
    let pad = spaces(indent);

    if let Some(doc) = &e.doc {
        push_doc_comment(out, doc, indent);
    }
    out.push_str(&format!("{pad}enum {} {{\n", e.name));

    for (i, variant) in e.variants.iter().enumerate() {
        out.push_str(&format!("{}    {} = {};\n", pad, variant, i));
    }

    out.push_str(&format!("{pad}}}\n\n"));
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

fn emit_message(out: &mut String, msg: &EmitMessage, indent: usize) {
    let pad = spaces(indent);

    if let Some(doc) = &msg.doc {
        push_doc_comment(out, doc, indent);
    }
    out.push_str(&format!("{pad}message {} {{\n", msg.name));

    // Nested definitions (enums and messages) before fields.
    for nested in &msg.nested {
        emit_def(out, nested, indent + 4);
    }
    if !msg.nested.is_empty() && !msg.fields.is_empty() {
        out.push('\n');
    }

    // Fields with sequential numbers.
    for (i, field) in msg.fields.iter().enumerate() {
        emit_field(out, field, i + 1, indent + 4);
    }

    out.push_str(&format!("{pad}}}\n\n"));
}

fn emit_field(out: &mut String, field: &EmitField, number: usize, indent: usize) {
    let pad = spaces(indent);
    let label = match field.mult {
        EmitMult::Repeated => "repeated ",
        EmitMult::Optional => "optional ",
        EmitMult::Required => "",
    };
    out.push_str(&format!(
        "{pad}{label}{} {} = {};\n",
        field.ty, field.name, number,
    ));
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

fn spaces(n: usize) -> String {
    " ".repeat(n)
}

/// Emit a block comment, wrapping each line of the doc string.
fn push_doc_comment(out: &mut String, doc: &str, indent: usize) {
    let pad = spaces(indent);
    out.push_str(&format!("{pad}// "));
    let first_line = doc.lines().next().unwrap_or("").trim();
    out.push_str(first_line);
    out.push('\n');
    for line in doc.lines().skip(1) {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            out.push_str(&format!("{pad}// {trimmed}\n"));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{EmitDef, EmitEnum, EmitField, EmitMessage, EmitModel, EmitMult};

    fn simple_model() -> EmitModel {
        EmitModel {
            sysml_name: "CotEventMsg".to_owned(),
            proto_package: "rsdk.cot.proto".to_owned(),
            xsd_namespace: "urn:rsdk:cot:proto".to_owned(),
            proto_imports: vec![],
            xsd_imports: vec![],
            definitions: vec![
                EmitDef::Message(EmitMessage {
                    name: "CotEvent".to_owned(),
                    doc: Some("A Cursor-on-Target event.".to_owned()),
                    nested: vec![],
                    fields: vec![
                        EmitField {
                            name: "type".to_owned(),
                            ty: "string".to_owned(),
                            mult: EmitMult::Required,
                            is_scalar: true,
                        },
                        EmitField {
                            name: "uid".to_owned(),
                            ty: "string".to_owned(),
                            mult: EmitMult::Required,
                            is_scalar: true,
                        },
                        EmitField {
                            name: "lat".to_owned(),
                            ty: "double".to_owned(),
                            mult: EmitMult::Optional,
                            is_scalar: true,
                        },
                    ],
                }),
                EmitDef::Enum(EmitEnum {
                    name: "Status".to_owned(),
                    doc: None,
                    variants: vec!["STATUS_UNKNOWN".to_owned(), "STATUS_ACTIVE".to_owned()],
                }),
            ],
        }
    }

    #[test]
    fn proto_contains_syntax() {
        let out = emit_proto(&simple_model());
        assert!(out.contains("syntax = \"proto3\";"), "missing syntax line");
        assert!(out.contains("package rsdk.cot.proto;"), "missing package");
    }

    #[test]
    fn proto_field_numbers() {
        let out = emit_proto(&simple_model());
        assert!(out.contains("string type = 1;"));
        assert!(out.contains("string uid = 2;"));
        assert!(out.contains("optional double lat = 3;"));
    }

    #[test]
    fn proto_enum_variants() {
        let out = emit_proto(&simple_model());
        assert!(out.contains("STATUS_UNKNOWN = 0;"));
        assert!(out.contains("STATUS_ACTIVE = 1;"));
    }
}
