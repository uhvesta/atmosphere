//! protoc-gen-prost: reads a CodeGeneratorRequest from stdin and writes the
//! CodeGeneratorResponse to stdout. Errors go in the response, where protoc
//! reports them.

use std::io::{self, Read, Write};

use prost::Message;
use protoc_gen_prost::GeneratorResultExt;

fn main() -> io::Result<()> {
    let mut request = Vec::new();
    io::stdin().read_to_end(&mut request)?;
    let response = protoc_gen_prost::execute(&request).unwrap_codegen_response();
    io::stdout().write_all(&response.encode_to_vec())
}
