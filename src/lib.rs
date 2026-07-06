//! Library surface for `sysml-v2-cli`'s lint/emit machinery, so other front
//! ends (e.g. the `nu_plugin_sysml_v2` Nushell plugin) can reuse it without
//! shelling out to the CLI binary.

pub mod emit_proto;
pub mod emit_xsd;
pub mod lint;
pub mod lower;
