//! The bridge MCP clients launch: stdio on one side, the running LowNotes on
//! the other. See `mcp_bridge.rs`.
fn main() {
    std::process::exit(lownotes_lib::mcp_bridge::run());
}
