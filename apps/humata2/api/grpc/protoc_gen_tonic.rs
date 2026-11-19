//! protoc-gen-tonic: reads a CodeGeneratorRequest from stdin, strips services
//! from imported (non-target) FileDescriptorProtos so each domain gRPC crate
//! generates ONLY its own services, and writes the CodeGeneratorResponse to
//! stdout. Errors go in the response, where protoc reports them.

use std::collections::HashSet;
use std::io::{self, Read, Write};

use prost::Message;
use prost_types::compiler::CodeGeneratorRequest;
use protoc_gen_prost::GeneratorResultExt;

fn main() -> io::Result<()> {
    let mut raw = Vec::new();
    io::stdin().read_to_end(&mut raw)?;

    let mut req = CodeGeneratorRequest::decode(raw.as_slice())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let to_generate: HashSet<&str> = req.file_to_generate.iter().map(String::as_str).collect();
    for file in &mut req.proto_file {
        let name = file.name.as_deref().unwrap_or("");
        if !to_generate.contains(name) {
            file.service.clear();
        }
    }

    let rewritten = req.encode_to_vec();
    let response = protoc_gen_tonic::execute(&rewritten).unwrap_codegen_response();
    io::stdout().write_all(&response.encode_to_vec())
}
