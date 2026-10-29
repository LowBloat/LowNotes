import { expect, test } from 'bun:test';
import { claudeCodeCommand, mcpServersConfig } from '../src/lib/mcp-config';

test('the client configuration names the bridge as a JSON string', () => {
  const path = 'C:\\Program Files\\LowNotes\\lownotes-mcp.exe';
  expect(JSON.parse(mcpServersConfig(path))).toEqual({ mcpServers: { lownotes: { command: path } } });
});

test('the Claude Code command quotes the bridge path for the platform shell', () => {
  expect(claudeCodeCommand('C:\\Program Files\\LowNotes\\lownotes-mcp.exe'))
    .toBe('claude mcp add --scope user lownotes -- "C:\\Program Files\\LowNotes\\lownotes-mcp.exe"');
  expect(claudeCodeCommand('/Applications/LowNotes.app/Contents/MacOS/lownotes-mcp'))
    .toBe("claude mcp add --scope user lownotes -- '/Applications/LowNotes.app/Contents/MacOS/lownotes-mcp'");
  expect(claudeCodeCommand("/home/o'neil/lownotes-mcp")).toBe("claude mcp add --scope user lownotes -- '/home/o'\\''neil/lownotes-mcp'");
});
