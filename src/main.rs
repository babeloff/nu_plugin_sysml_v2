use nu_plugin::{EngineInterface, EvaluatedCall, MsgPackSerializer, Plugin, PluginCommand, serve_plugin};
use nu_protocol::{record, Category, LabeledError, Signature, Span, Type, Value};
use sysml_v2_parser::{parse, parse_for_editor, ParseError};

struct SysMlPlugin;

impl Plugin for SysMlPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(FromSysml)]
    }
}

struct FromSysml;

impl PluginCommand for FromSysml {
    type Plugin = SysMlPlugin;

    fn name(&self) -> &str {
        "from sysml"
    }

    fn description(&self) -> &str {
        "Parse SysML v2 textual notation into structured data"
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self))
            .input_output_type(Type::String, Type::Any)
            .switch(
                "resilient",
                "Return a partial AST alongside syntax errors instead of failing on the first error",
                Some('r'),
            )
            .category(Category::Formats)
    }

    fn run(
        &self,
        _plugin: &SysMlPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        input: nu_protocol::PipelineData,
    ) -> Result<nu_protocol::PipelineData, LabeledError> {
        let span = call.head;
        let input_span = input.span().unwrap_or(span);
        let input_value = input.into_value(input_span)?;
        let source = input_value.as_str().map_err(|err| {
            LabeledError::new("Expected string input").with_label(err.to_string(), span)
        })?;

        let resilient = call.has_flag("resilient")?;

        let value = if resilient {
            let result = parse_for_editor(source);
            let root_value = json_to_nu_value(
                serde_json::to_value(&result.root).map_err(|e| {
                    LabeledError::new("Failed to serialize AST").with_label(e.to_string(), span)
                })?,
                span,
            );
            let errors: Vec<Value> = result.errors.iter().map(|e| parse_error_to_value(e, span)).collect();
            Value::record(
                record! {
                    "root" => root_value,
                    "errors" => Value::list(errors, span),
                },
                span,
            )
        } else {
            match parse(source) {
                Ok(root) => {
                    let json = serde_json::to_value(&root).map_err(|e| {
                        LabeledError::new("Failed to serialize AST").with_label(e.to_string(), span)
                    })?;
                    json_to_nu_value(json, span)
                }
                Err(e) => {
                    return Err(LabeledError::new("SysML parse error")
                        .with_label(e.message.clone(), span));
                }
            }
        };

        Ok(nu_protocol::PipelineData::value(value, None))
    }
}

fn parse_error_to_value(err: &ParseError, span: Span) -> Value {
    Value::record(
        record! {
            "message" => Value::string(err.message.clone(), span),
            "line" => match err.line {
                Some(l) => Value::int(l as i64, span),
                None => Value::nothing(span),
            },
            "column" => match err.column {
                Some(c) => Value::int(c as i64, span),
                None => Value::nothing(span),
            },
            "expected" => match &err.expected {
                Some(s) => Value::string(s.clone(), span),
                None => Value::nothing(span),
            },
            "found" => match &err.found {
                Some(s) => Value::string(s.clone(), span),
                None => Value::nothing(span),
            },
            "suggestion" => match &err.suggestion {
                Some(s) => Value::string(s.clone(), span),
                None => Value::nothing(span),
            },
        },
        span,
    )
}

/// Convert a generic `serde_json::Value` (produced by serializing the SysML AST) into a Nu
/// `Value`, so the AST's shape drives the Nu record/list structure without a hand-written
/// match arm per AST node type.
fn json_to_nu_value(json: serde_json::Value, span: Span) -> Value {
    match json {
        serde_json::Value::Null => Value::nothing(span),
        serde_json::Value::Bool(b) => Value::bool(b, span),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::int(i, span)
            } else {
                Value::float(n.as_f64().unwrap_or(0.0), span)
            }
        }
        serde_json::Value::String(s) => Value::string(s, span),
        serde_json::Value::Array(items) => {
            let values = items.into_iter().map(|v| json_to_nu_value(v, span)).collect();
            Value::list(values, span)
        }
        serde_json::Value::Object(map) => {
            let record = map
                .into_iter()
                .map(|(k, v)| (k, json_to_nu_value(v, span)))
                .collect();
            Value::record(record, span)
        }
    }
}

fn main() {
    serve_plugin(&SysMlPlugin, MsgPackSerializer)
}
