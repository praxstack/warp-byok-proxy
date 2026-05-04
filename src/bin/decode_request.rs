//! Decode a captured `warp_multi_agent_api::Request` protobuf from disk and
//! print structure. Used to audit real-world Warp requests against the shape
//! our proxy expects.

use prost::Message;
use std::path::Path;
use warp_multi_agent_api as wmaa;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/warp-multi-agent-request.bin".to_string());
    let bytes = std::fs::read(Path::new(&path))?;
    println!("[info] read {} bytes from {}", bytes.len(), path);
    let req = wmaa::Request::decode(bytes.as_ref())?;

    if let Some(md) = &req.metadata {
        println!("--- metadata ---");
        println!("{:#?}", md);
    } else {
        println!("(no metadata)");
    }

    println!();
    println!("--- input top-level ---");
    if let Some(inp) = &req.input {
        println!("{:#?}", inp);
    } else {
        println!("(no input)");
    }

    println!();
    println!("--- full request dump (trimmed) ---");
    let dump = format!("{:#?}", req);
    let total = dump.lines().count();
    for (i, line) in dump.lines().enumerate() {
        if i < 600 {
            println!("{}", line);
        } else {
            println!("... [{} more lines]", total - 600);
            break;
        }
    }
    Ok(())
}
