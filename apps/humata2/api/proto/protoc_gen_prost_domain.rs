//! protoc-gen-prost-domain: wraps `protoc-gen-prost` for same-package
//! (`package humata.v1;`) domain crates.
//!
//! Reads a `CodeGeneratorRequest` from stdin, extracts `dep=<proto_path>=<crate_name>`
//! options from `request.parameter`, and for every imported `FileDescriptorProto`
//! in `package humata.v1` that is not in `file_to_generate`, injects
//! `extern_path=.humata.v1.<Type>=::<crate_name>::humata::v1::<Type>` for each
//! top-level message and enum. This ensures every message/enum is emitted in
//! exactly one domain crate and all cross-domain references share nominal Rust
//! type identity.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};

use prost::Message;
use prost_types::compiler::CodeGeneratorRequest;
use protoc_gen_prost::GeneratorResultExt;

fn main() -> io::Result<()> {
    let mut raw = Vec::new();
    io::stdin().read_to_end(&mut raw)?;

    let mut req = CodeGeneratorRequest::decode(raw.as_slice())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut dep_map: HashMap<String, String> = HashMap::new();
    let mut params: Vec<String> = Vec::new();

    if let Some(param_str) = req.parameter.take() {
        for part in param_str.split(',').filter(|s| !s.is_empty()) {
            if let Some(rest) = part.strip_prefix("dep=") {
                if let Some((proto_path, crate_name)) = rest.split_once('=') {
                    dep_map.insert(proto_path.to_owned(), crate_name.to_owned());
                    continue;
                }
            }
            params.push(part.to_owned());
        }
    }

    let to_generate: HashSet<&str> = req.file_to_generate.iter().map(String::as_str).collect();

    for file in &req.proto_file {
        let name = file.name.as_deref().unwrap_or("");
        if to_generate.contains(name) {
            continue;
        }
        let pkg = file.package.as_deref().unwrap_or("");
        if pkg != "humata.v1" {
            continue;
        }
        let Some(crate_name) = dep_map.get(name) else {
            continue;
        };
        for msg in &file.message_type {
            if let Some(msg_name) = msg.name.as_deref() {
                params.push(format!(
                    "extern_path=.humata.v1.{msg_name}=::{crate_name}::humata::v1::{msg_name}"
                ));
            }
        }
        for en in &file.enum_type {
            if let Some(en_name) = en.name.as_deref() {
                params.push(format!(
                    "extern_path=.humata.v1.{en_name}=::{crate_name}::humata::v1::{en_name}"
                ));
            }
        }
    }

    if !params.is_empty() {
        req.parameter = Some(params.join(","));
    }

    let rewritten = req.encode_to_vec();
    let response = protoc_gen_prost::execute(&rewritten).unwrap_codegen_response();
    io::stdout().write_all(&response.encode_to_vec())
}
