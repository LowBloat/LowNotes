/** Configuration that points an MCP client at the `lownotes-mcp` bridge. */

/** `mcpServers` entry for Claude Desktop and Cursor, which share the format. */
export function mcpServersConfig(bridgePath: string): string {
  return JSON.stringify({ mcpServers: { lownotes: { command: bridgePath } } }, null, 2);
}

/** Claude Code command that registers the bridge for every project. */
export function claudeCodeCommand(bridgePath: string): string {
  return `claude mcp add --scope user lownotes -- ${shellQuote(bridgePath)}`;
}

/** Windows paths go in double quotes (cmd and PowerShell); others in single quotes. */
function shellQuote(path: string): string {
  if (/^[A-Za-z]:\\/.test(path)) return `"${path}"`;
  return `'${path.replace(/'/g, `'\\''`)}'`;
}
