const fs = require('fs'), path = require('path');
const tools = ['mcp__dx__dx_search', 'mcp__dx__dx_source', 'mcp__dx__dx_read', 'mcp__dx__dx_outline', 'mcp__dx__dx_edit', 'mcp__dx__dx_write', 'mcp__dx__dx_append', 'mcp__dx__dx_check', 'mcp__dx__dx_list', 'mcp__dx__dx_board', 'mcp__dx__dx_run', 'mcp__dx__dx_sync', 'mcp__dx__dx_coverage', 'mcp__dx__dx_report', 'mcp__dx__dx_index', 'mcp__dx__dx_play', 'mcp__dx__dx_render'];
const p = path.join(process.env.HOME, '.claude', 'settings.json');
let s = JSON.parse(fs.readFileSync(p, 'utf8'));
const allow = new Set(s.permissions?.allow || []), ask = new Set(s.permissions?.ask || []), deny = new Set(s.permissions?.deny || []);
tools.forEach(t => { allow.add(t); ask.delete(t); deny.delete(t); });
s.permissions = { defaultMode: s.permissions?.defaultMode || 'auto', allow: Array.from(allow).sort(), ask: Array.from(ask).sort(), ...(deny.size > 0 && { deny: Array.from(deny).sort() }) };
fs.writeFileSync(p, JSON.stringify(s, null, 2) + '\n');
console.log(`✓ DX: ${allow.size} allow`);
