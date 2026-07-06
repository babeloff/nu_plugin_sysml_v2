//! EmitModel → XML Schema Definition (XSD) text.
//!
//! Produces a single `.xsd` file for one SysML package.  The output targets
//! XML Schema 1.0 (`http://www.w3.org/2001/XMLSchema`) and mirrors what the
//! hand-written `protos/*.proto` files represent, but in XSD form.
//!
//! Mapping conventions
//! ───────────────────
//! • `EmitMessage`   → `xs:complexType` (sequence of child elements)
//! • `EmitEnum`      → `xs:simpleType` (restriction on `xs:string` with `xs:enumeration`)
//! • Required field  → `xs:element minOccurs="1" maxOccurs="1"`  (default, omitted)
//! • Optional field  → `xs:element minOccurs="0" maxOccurs="1"`
//! • Repeated field  → `xs:element minOccurs="0" maxOccurs="unbounded"`
//! • Proto scalars   → mapped to `xs:string`, `xs:long`, `xs:double`, `xs:boolean`
//! • Message refs    → typed to the local or imported complex type name

use crate::lower::{EmitDef, EmitEnum, EmitField, EmitMessage, EmitModel, EmitMult};

const XS: &str = "http://www.w3.org/2001/XMLSchema";

/// Render an [`EmitModel`] as XSD source text.
pub fn emit_xsd(model: &EmitModel) -> String {
    let mut out = String::new();

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!("<xs:schema xmlns:xs=\"{XS}\"\n"));
    out.push_str(&format!(
        "           targetNamespace=\"{}\"\n",
        model.xsd_namespace
    ));
    out.push_str(&format!("           xmlns=\"{}\"\n", model.xsd_namespace));
    out.push_str("           elementFormDefault=\"qualified\"");

    // Namespace declarations for imported schemas
    for (i, (ns, _loc)) in model.xsd_imports.iter().enumerate() {
        out.push_str(&format!("\n           xmlns:ns{i}=\"{ns}\""));
    }
    out.push_str(">\n\n");

    // xs:import for each cross-package reference
    for (ns, loc) in &model.xsd_imports {
        out.push_str(&format!(
            "  <xs:import namespace=\"{ns}\" schemaLocation=\"{loc}\"/>\n"
        ));
    }
    if !model.xsd_imports.is_empty() {
        out.push('\n');
    }

    // Definitions
    for def in &model.definitions {
        emit_def(&mut out, def, 2);
    }

    out.push_str("</xs:schema>\n");
    out
}

// ---------------------------------------------------------------------------
// Definition dispatch
// ---------------------------------------------------------------------------

fn emit_def(out: &mut String, def: &EmitDef, indent: usize) {
    match def {
        EmitDef::Enum(e) => emit_simple_type(out, e, indent),
        EmitDef::Message(m) => emit_complex_type(out, m, indent),
    }
}

// ---------------------------------------------------------------------------
// simpleType (enum)
// ---------------------------------------------------------------------------

fn emit_simple_type(out: &mut String, e: &EmitEnum, indent: usize) {
    let pad = spaces(indent);
    let pad2 = spaces(indent + 2);
    let pad4 = spaces(indent + 4);

    if let Some(doc) = &e.doc {
        push_annotation(out, doc, indent);
    }

    out.push_str(&format!("{pad}<xs:simpleType name=\"{}\">\n", e.name));
    out.push_str(&format!("{pad2}<xs:restriction base=\"xs:string\">\n"));
    for variant in &e.variants {
        out.push_str(&format!("{pad4}<xs:enumeration value=\"{variant}\"/>\n"));
    }
    out.push_str(&format!("{pad2}</xs:restriction>\n"));
    out.push_str(&format!("{pad}</xs:simpleType>\n\n"));
}

// ---------------------------------------------------------------------------
// complexType (message)
// ---------------------------------------------------------------------------

fn emit_complex_type(out: &mut String, msg: &EmitMessage, indent: usize) {
    let pad = spaces(indent);
    let pad2 = spaces(indent + 2);

    if let Some(doc) = &msg.doc {
        push_annotation(out, doc, indent);
    }

    out.push_str(&format!("{pad}<xs:complexType name=\"{}\">\n", msg.name));
    out.push_str(&format!("{pad2}<xs:sequence>\n"));

    for field in &msg.fields {
        emit_element(out, field, indent + 4);
    }

    out.push_str(&format!("{pad2}</xs:sequence>\n"));

    // Nested definitions go inside the complexType as annotations.
    // XSD 1.0 does not support nested type definitions, so we emit them as
    // top-level siblings.  We collect them and they will be emitted separately
    // via the `emit_nested` helper called from `emit_complex_type_with_nested`.
    out.push_str(&format!("{pad}</xs:complexType>\n\n"));

    // Emit nested definitions as sibling top-level types (XSD 1.0 limitation).
    for nested in &msg.nested {
        emit_def(out, nested, indent);
    }
}

fn emit_element(out: &mut String, field: &EmitField, indent: usize) {
    let pad = spaces(indent);
    let xsd_type = proto_to_xsd_type(&field.ty, field.is_scalar);

    let (min, max) = match field.mult {
        EmitMult::Required => ("1", "1"),
        EmitMult::Optional => ("0", "1"),
        EmitMult::Repeated => ("0", "unbounded"),
    };

    // Omit minOccurs/maxOccurs when both are the default (1,1).
    if min == "1" && max == "1" {
        out.push_str(&format!(
            "{pad}<xs:element name=\"{}\" type=\"{xsd_type}\"/>\n",
            field.name
        ));
    } else {
        out.push_str(&format!(
            "{pad}<xs:element name=\"{}\" type=\"{xsd_type}\" minOccurs=\"{min}\" maxOccurs=\"{max}\"/>\n",
            field.name
        ));
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn proto_to_xsd_type(proto_ty: &str, is_scalar: bool) -> String {
    if is_scalar {
        let xsd = match proto_ty {
            "string" => "xs:string",
            "int32" => "xs:int",
            "int64" => "xs:long",
            "uint32" => "xs:unsignedInt",
            "uint64" => "xs:unsignedLong",
            "float" => "xs:float",
            "double" => "xs:double",
            "bool" => "xs:boolean",
            "bytes" => "xs:base64Binary",
            _ => "xs:string",
        };
        xsd.to_owned()
    } else {
        // Non-scalar: use the type name as-is (refers to local complex/simple type).
        proto_ty.to_owned()
    }
}

fn spaces(n: usize) -> String {
    " ".repeat(n)
}

fn push_annotation(out: &mut String, doc: &str, indent: usize) {
    let pad = spaces(indent);
    out.push_str(&format!("{pad}<xs:annotation>\n"));
    out.push_str(&format!("{pad}  <xs:documentation>\n"));
    for line in doc.lines() {
        let t = line.trim();
        if !t.is_empty() {
            out.push_str(&format!("{pad}    {t}\n"));
        }
    }
    out.push_str(&format!("{pad}  </xs:documentation>\n"));
    out.push_str(&format!("{pad}</xs:annotation>\n"));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{EmitDef, EmitField, EmitMessage, EmitModel, EmitMult};

    fn simple_model() -> EmitModel {
        EmitModel {
            sysml_name: "CotEventMsg".to_owned(),
            proto_package: "rsdk.cot.proto".to_owned(),
            xsd_namespace: "urn:rsdk:cot:proto".to_owned(),
            proto_imports: vec![],
            xsd_imports: vec![],
            definitions: vec![EmitDef::Message(EmitMessage {
                name: "CotEvent".to_owned(),
                doc: None,
                nested: vec![],
                fields: vec![
                    EmitField {
                        name: "type".to_owned(),
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
            })],
        }
    }

    #[test]
    fn xsd_has_header() {
        let out = emit_xsd(&simple_model());
        assert!(out.contains("<?xml version=\"1.0\""));
        assert!(out.contains("xs:schema"));
        assert!(out.contains("urn:rsdk:cot:proto"));
    }

    #[test]
    fn xsd_complex_type() {
        let out = emit_xsd(&simple_model());
        assert!(out.contains("<xs:complexType name=\"CotEvent\">"));
        assert!(out.contains("<xs:element name=\"type\" type=\"xs:string\"/>"));
        assert!(out.contains("minOccurs=\"0\" maxOccurs=\"1\""));
    }
}
