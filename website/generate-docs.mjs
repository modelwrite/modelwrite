// SPDX-License-Identifier: AGPL-3.0-or-later
// Regenerates the marked, machine-written sections of the documentation from
// their sources of truth. A section is delimited by
//
//     <!-- generated:NAME -->   ...   <!-- /generated -->
//
// so a reader can see which parts are generated and must not be hand-edited.
// Hand-editing the content between the markers is pointless: this script (run by
// CI) rewrites it and the drift check fails the build.
//
// Sources of truth, always read from the repository, never from prose:
//   - crate licences      -> every workspace Cargo.toml (the [workspace.package]
//                            default plus any per-crate override)
//   - binding list        -> each binding crate's BINDING_ID / BINDING_VERSION /
//                            Direction::* / description, plus the server's binding
//                            registry (the workbench's actual import surface)
//   - MCP tool list       -> docs/agents/mcp-tools.json (contract-enforced
//                            against the live tool table)
//   - metric definitions  -> spec/analytics/metric_definitions.json (referenced,
//                            never duplicated)
//
// Usage: node website/generate-docs.mjs   (rewrites the marked sections in place)
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(here, '..');

const read = (rel) =>
  fs.readFileSync(path.join(ROOT, rel), 'utf8').replace(/\r\n/g, '\n');

// ---------------------------------------------------------------------------
// Workspace members, from the root Cargo.toml [workspace] table.
// ---------------------------------------------------------------------------
function workspaceMembers() {
  const rootToml = read('Cargo.toml');
  const raw = (rootToml.match(/^members\s*=\s*\[([^\]]*)\]/m) || [])[1];
  if (!raw) throw new Error('workspace members not found in Cargo.toml');
  return raw
    .split(',')
    .map((s) => s.trim().replace(/^"|"$/g, ''))
    .filter(Boolean);
}

// ---------------------------------------------------------------------------
// crate licences
// ---------------------------------------------------------------------------
function crateLicences() {
  const rootToml = read('Cargo.toml');
  const wsLicense = (rootToml.match(/^license\s*=\s*"([^"]+)"/m) || [])[1];
  if (!wsLicense) throw new Error('workspace license not found in Cargo.toml');

  const rows = workspaceMembers().map((dir) => {
    const toml = read(path.join(dir, 'Cargo.toml'));
    const name = (toml.match(/^name\s*=\s*"([^"]+)"/m) || [])[1];
    if (!name) throw new Error('[package] name not found in ' + dir + '/Cargo.toml');
    let license;
    if (/^license\.workspace\s*=\s*true/m.test(toml)) {
      license = wsLicense;
    } else {
      license = (toml.match(/^license\s*=\s*"([^"]+)"/m) || [])[1];
      if (!license) throw new Error('license not declared in ' + dir + '/Cargo.toml');
    }
    return { name, license };
  });

  rows.sort((a, b) => a.name.localeCompare(b.name));

  const lines = ['| Crate | Licence |', '|---|---|'];
  for (const r of rows) lines.push('| `' + r.name + '` | ' + r.license + ' |');
  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// binding list
// ---------------------------------------------------------------------------
function bindings() {
  const out = [];
  for (const dir of workspaceMembers()) {
    const libPath = path.join(dir, 'src', 'lib.rs');
    if (!fs.existsSync(path.join(ROOT, libPath))) continue;
    const src = read(libPath);
    const id = (src.match(/pub const BINDING_ID:\s*&str\s*=\s*"([^"]+)"/) || [])[1];
    if (!id) continue; // the mw-binding trait crate has no BINDING_ID
    const version = (src.match(/pub const BINDING_VERSION:\s*&str\s*=\s*"([^"]+)"/) || [])[1];
    const direction = (src.match(/direction:\s*Direction::(\w+)/) || [])[1];
    const description = (src.match(/description:\s*"([^"]*)"/s) || [])[1];
    if (!version || !direction || !description) {
      throw new Error('binding info incomplete in ' + libPath);
    }
    const toml = read(path.join(dir, 'Cargo.toml'));
    const libName = (toml.match(/\[lib\][\s\S]*?^name\s*=\s*"([^"]+)"/m) || [])[1];
    if (!libName) throw new Error('[lib] name not found in ' + dir + '/Cargo.toml');
    out.push({ id, version, direction, description, libName });
  }
  out.sort((a, b) => a.id.localeCompare(b.id));
  return out;
}

function registryLibNames() {
  const src = read('server/src/binding_registry.rs');
  const names = new Set();
  const re = /\b(binding_[A-Za-z0-9_]+)::/g;
  let m;
  while ((m = re.exec(src))) names.add(m[1]);
  return names;
}

function splitDescription(desc) {
  const i = desc.indexOf(': ');
  if (i === -1) return [desc, desc];
  return [desc.slice(0, i), desc.slice(i + 2)];
}

function renderBindings() {
  const list = bindings();
  const registry = registryLibNames();

  const lines = ['| Binding | id@version | Direction | Reads |', '|---|---|---|---|'];
  for (const b of list) {
    const dirLabel = b.direction === 'ImportAndExport' ? 'read/write' : 'viewer (ImportOnly)';
    const [label, reads] = splitDescription(b.description);
    lines.push('| ' + label + ' | `' + b.id + '@' + b.version + '` | ' + dirLabel + ' | ' + reads + ' |');
  }

  const registered = list.filter((b) => registry.has(b.libName));
  const unregistered = list.filter((b) => !registry.has(b.libName));
  const regIds = registered.map((b) => '`' + b.id + '@' + b.version + '`').join(' and ');
  let note;
  if (registered.length === 0) {
    note = "The workbench import surface (the server's binding registry) offers no binding.";
  } else {
    note = "The workbench import surface (the server's binding registry) offers " + regIds;
    if (unregistered.length > 0) {
      const un = unregistered
        .map((b) => '`' + b.id + '@' + b.version + '`')
        .join(' and ');
      note += '; ' + un + ' ' + (unregistered.length === 1 ? 'is' : 'are') +
        ' not registered in the server, so the workbench does not offer ' +
        (unregistered.length === 1 ? 'it' : 'them') + '.';
    } else {
      note += '.';
    }
  }

  lines.push('', note);
  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// MCP tool list
// ---------------------------------------------------------------------------
function renderMcpTools() {
  const data = JSON.parse(read('docs/agents/mcp-tools.json'));
  const tools = data.tools;
  if (!Array.isArray(tools) || tools.length === 0) {
    throw new Error('docs/agents/mcp-tools.json has no tools');
  }
  const clean = (s) => String(s).replace(/\r?\n/g, ' ').replace(/\|/g, '\\|');
  const lines = [
    'The MCP server exposes **' + tools.length + '** tools. The names and schemas are contract-enforced against the live tool table and live in [docs/agents/mcp-tools.json](docs/agents/mcp-tools.json):',
    '',
    '| Tool | Required arguments | Summary |',
    '|---|---|---|',
  ];
  for (const t of tools) {
    const required = (t.inputSchema && t.inputSchema.required) || [];
    const args = required.length ? required.join(', ') : '—';
    lines.push('| `' + t.name + '` | ' + args + ' | ' + clean(t.summary) + ' |');
  }
  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// metric definitions (referenced, not duplicated)
// ---------------------------------------------------------------------------
function renderMetrics() {
  const metrics = JSON.parse(read('spec/analytics/metric_definitions.json'));
  if (!Array.isArray(metrics)) throw new Error('metric_definitions.json is not an array');
  return (
    metrics.length +
    " metric definitions, listed in [spec/analytics/metric_definitions.json](spec/analytics/metric_definitions.json); the website's analytics section renders the same file, so the list is never hand-duplicated."
  );
}

// ---------------------------------------------------------------------------
// apply a generated section into a file
// ---------------------------------------------------------------------------
function applySection(rel, name, content) {
  const open = '<!-- generated:' + name + ' -->';
  const close = '<!-- /generated -->';
  const text = read(rel);
  const openIdx = text.indexOf(open);
  if (openIdx === -1) throw new Error('marker ' + open + ' not found in ' + rel);
  const closeIdx = text.indexOf(close, openIdx + open.length);
  if (closeIdx === -1) throw new Error('closing marker ' + close + ' not found after ' + open + ' in ' + rel);
  const before = text.slice(0, openIdx + open.length);
  const after = text.slice(closeIdx);
  const next = before + '\n' + content + '\n' + after;
  if (next !== text) {
    fs.writeFileSync(path.join(ROOT, rel), next, 'utf8');
  }
}

// ---------------------------------------------------------------------------
// the fixed set of generated sections, by file
// ---------------------------------------------------------------------------
const SECTIONS = [
  ['README.md', 'binding-list', renderBindings],
  ['README.md', 'mcp-tools', renderMcpTools],
  ['README.md', 'metric-definitions', renderMetrics],
  ['README.md', 'crate-licences', crateLicences],
  ['docs/guide/limits.md', 'binding-list', renderBindings],
  ['docs/guide/faq.md', 'binding-list', renderBindings],
  ['docs/guide/mcp-agents.md', 'mcp-tools', renderMcpTools],
];

let changed = 0;
for (const [file, name, render] of SECTIONS) {
  const before = read(file);
  applySection(file, name, render());
  if (read(file) !== before) changed++;
}

console.log('generated sections refreshed; files changed: ' + changed);
