const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };

const state = { running: false, toolNodes: new Map(), selectedFile: null, filesDir: '' };

// ---------- boot ----------
(async () => {
  try {
    const cfg = await invoke('get_config');
    $('workdir').value = cfg.cwd || cfg.home;
    $('maxTurns').value = cfg.agent.max_turns;
    $('net').checked = cfg.net.enabled;
    $('serverInfo').textContent = cfg.llm.base_url;
    const models = await invoke('list_models').catch(e => { addSystem('models: ' + e); return [cfg.llm.model]; });
    for (const m of models) { const o = el('option', null, m); o.value = m; if (m === cfg.llm.model) o.selected = true; $('model').appendChild(o); }
    if (!models.includes(cfg.llm.model)) { const o = el('option', null, cfg.llm.model + ' (configured)'); o.value = cfg.llm.model; o.selected = true; $('model').prepend(o); }
    refreshFiles(); refreshGit();
  } catch (e) { addError('config: ' + e); }
})();

// ---------- events from the core ----------
listen('agent-event', ({ payload: e }) => {
  switch (e.type) {
    case 'run_started': addSystem(`model ${e.model} · ${e.tools.length} tools · ${e.workdir}`); break;
    case 'turn': setStats(`turn ${e.n}`); break;
    case 'reasoning': addReasoning(e.text); break;
    case 'assistant': addBubble('assistant', e.text); break;
    case 'tool_call': addToolCall(e); break;
    case 'tool_result': finishToolCall(e); refreshFiles(); break;
    case 'compacted': addSystem(`compacted ${e.count} old tool results (prompt was ${e.prompt_tokens} tokens)`); break;
    case 'run_finished': addFinished(e); break;
    case 'error': addError(e.message); break;
    case 'memory': addSystem(`🧠 ${e.file} › ${e.section}: ${e.text}`); break;
    case 'permission': if ((e.decision || '').startsWith('denied')) addError(`🔒 ${e.tool}: ${e.decision}`); break;
    case 'model_response': setStats(`${e.completion_tokens} tok · ttft ${e.ttft_secs.toFixed(1)}s · ${(e.completion_tokens / Math.max(0.1, e.secs - e.ttft_secs)).toFixed(1)} tok/s`); break;
    case 'assistant_delta': case 'reasoning_delta': break;
  }
});
listen('permission-ask', ({ payload: p }) => {
  const d = el('div', 'ev tool'); d.style.borderColor = 'var(--accent)';
  d.appendChild(el('div', null, `🔒 ${p.tool}(${p.summary}) — ${p.reason}`));
  const row = el('div', 'row'); row.style.marginTop = '8px';
  const mk = (label, dec, cls) => { const b = el('button', cls, label); b.onclick = async () => { await invoke('answer_permission', { id: p.id, decision: dec }); d.querySelectorAll('button').forEach(x => x.disabled = true); d.appendChild(el('div', 'dim', `→ ${label}`)); }; return b; };
  row.appendChild(mk('Allow once', 'once', 'primary')); row.appendChild(mk(`Always (${p.rule})`, 'always', '')); row.appendChild(mk('Deny', 'deny', ''));
  d.appendChild(row); tl().appendChild(d); scrollDown();
});
listen('run-finished', ({ payload: p }) => {
  state.running = false; $('run').disabled = false; $('stop').disabled = true;
  if (p.ok) { if (p.text && p.text.trim()) addBubble('assistant', p.text); $('composerHint').textContent = 'Done.'; }
  else { addError(p.error || 'run failed'); $('composerHint').textContent = 'Stopped.'; }
  refreshFiles(); refreshGit();
});

// ---------- run controls ----------
async function startRun() {
  const task = $('task').value.trim(); if (!task) return;
  const workdir = $('workdir').value.trim();
  $('timeline').innerHTML = ''; state.toolNodes.clear();
  addBubble('user', task);
  try {
    await invoke('start_run', { task, workdir, model: $('model').value, maxTurns: +$('maxTurns').value || null, net: $('net').checked });
    state.running = true; $('run').disabled = true; $('stop').disabled = false;
    $('composerHint').textContent = 'Running…'; setStats('');
    state.filesDir = workdir; refreshFiles();
  } catch (e) { addError(String(e)); }
}
$('run').onclick = startRun;
$('stop').onclick = () => invoke('stop_run');
$('task').addEventListener('keydown', (ev) => { if ((ev.metaKey || ev.ctrlKey) && ev.key === 'Enter') startRun(); });
$('pickDir').onclick = async () => {
  try {
    const dir = await window.__TAURI__.dialog.open({ directory: true, multiple: false, defaultPath: $('workdir').value });
    if (dir) { $('workdir').value = dir; state.filesDir = dir; refreshFiles(); refreshGit(); }
  } catch (e) { addSystem('dialog: ' + e); }
};
$('workdir').addEventListener('change', () => { state.filesDir = $('workdir').value; refreshFiles(); refreshGit(); });
$('filesRefresh').onclick = () => refreshFiles();
$('gitRefresh').onclick = () => refreshGit();

// ---------- timeline ----------
const tl = () => $('timeline');
function scrollDown() { const t = tl(); t.scrollTop = t.scrollHeight; }
function addBubble(kind, text) { tl().appendChild(el('div', 'ev ' + kind, text)); scrollDown(); }
function addSystem(text) { tl().appendChild(el('div', 'ev system', text)); scrollDown(); }
function addError(text) { tl().appendChild(el('div', 'ev error', text)); scrollDown(); }
function addFinished(e) {
  tl().appendChild(el('div', 'ev finished', `✓ ${e.stop_reason} · ${e.turns} turns · ${e.tool_calls} tool calls · ${e.prompt_tokens}+${e.completion_tokens} tokens · ${e.wall_secs.toFixed(0)}s`));
  setStats(`${e.turns} turns · ${e.tool_calls} tools · ${e.prompt_tokens}+${e.completion_tokens} tok · ${e.wall_secs.toFixed(0)}s`);
  scrollDown();
}
function addReasoning(text) {
  const d = el('details', 'ev reasoning'); const s = el('summary', null, '💭 ' + text.trim().split('\n')[0].slice(0, 140));
  d.appendChild(s); d.appendChild(el('pre', null, text.trim())); tl().appendChild(d); scrollDown();
}
function prettyArgs(args) { try { const o = JSON.parse(args); return Object.entries(o).map(([k, v]) => `${k}=${typeof v === 'string' ? JSON.stringify(v.length > 80 ? v.slice(0, 80) + '…' : v) : JSON.stringify(v)}`).join(' '); } catch { return args; } }
function fullArgs(args) { try { const o = JSON.parse(args); return Object.entries(o).map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v, null, 2)}`).join('\n\n'); } catch { return args; } }
function addToolCall(e) {
  const d = el('details', 'ev tool pending'); d.dataset.id = e.id;
  const s = el('summary'); s.appendChild(el('span', 'name', e.name)); s.appendChild(el('span', 'args', prettyArgs(e.args))); s.appendChild(el('span', 'secs', ''));
  d.appendChild(s);
  d.appendChild(el('pre', 'args', fullArgs(e.args)));
  tl().appendChild(d); state.toolNodes.set(e.id, d); scrollDown();
}
function finishToolCall(e) {
  const d = state.toolNodes.get(e.id); if (!d) return;
  d.classList.remove('pending');
  d.querySelector('.secs').textContent = `${e.secs.toFixed(1)}s`;
  d.appendChild(el('pre', null, e.result));
  if (e.result.startsWith('error:')) d.style.borderColor = '#4a2222';
  if (e.images && e.images.length) { const w = el('div', 'imgs'); for (const src of e.images) { const img = el('img'); img.src = src; w.appendChild(img); } d.appendChild(w); d.open = true; }
  scrollDown();
}
function setStats(t) { $('stats').textContent = t; }

// ---------- files & preview ----------
async function refreshFiles(dir) {
  dir = dir || state.filesDir || $('workdir').value;
  state.filesDir = dir;
  $('fileRoot').textContent = dir.replace(/^\/Users\/[^/]+/, '~');
  try {
    const entries = await invoke('list_dir', { path: dir });
    const box = $('files'); box.innerHTML = '';
    const root = $('workdir').value;
    if (dir !== root && dir.startsWith(root)) { const up = el('div', 'f dir', '↑ ..'); up.onclick = () => refreshFiles(dir.replace(/\/[^/]+$/, '') || root); box.appendChild(up); }
    for (const en of entries) {
      const row = el('div', 'f' + (en.is_dir ? ' dir' : '') + (state.selectedFile === en.path ? ' sel' : ''));
      row.dataset.path = en.path;
      row.appendChild(el('span', null, (en.is_dir ? '▸ ' : '') + en.name));
      if (!en.is_dir) row.appendChild(el('span', 'sz', human(en.size)));
      row.onclick = () => en.is_dir ? refreshFiles(en.path) : preview(en.path);
      box.appendChild(row);
    }
    if (state.selectedFile && state.selectedFile.startsWith(dir + '/')) preview(state.selectedFile, true);
  } catch (e) { $('files').innerHTML = ''; $('files').appendChild(el('div', 'f dim', String(e))); }
}
async function preview(path, silent) {
  state.selectedFile = path;
  for (const r of document.querySelectorAll('.f')) r.classList.toggle('sel', r.dataset.path === path);
  const box = $('preview'); if (!silent) box.innerHTML = '';
  try {
    const p = await invoke('read_file', { path });
    box.innerHTML = '';
    box.appendChild(el('div', 'ph', `${path.split('/').pop()} · ${p.kind} · ${human(p.size)}`));
    if (p.kind === 'text') box.appendChild(el('pre', null, p.text));
    else if (p.kind === 'image') { const i = el('img'); i.src = p.data_url; box.appendChild(i); }
    else if (p.kind === 'audio') { const a = el('audio'); a.controls = true; a.src = p.data_url; box.appendChild(a); }
    else if (p.kind === 'video') { const v = el('video'); v.controls = true; v.src = p.data_url; box.appendChild(v); }
    else if (p.kind === 'pdf') { const f = el('iframe'); f.src = p.data_url; f.style.cssText = 'width:100%;height:80vh;border:0'; box.appendChild(f); }
    else box.appendChild(el('div', 'dim', 'binary file — no preview'));
  } catch (e) { box.appendChild(el('div', 'dim', String(e))); }
}
async function refreshGit() {
  try { $('gitLog').textContent = (await invoke('git_log', { workdir: $('workdir').value })).trim() || '(not a git repo)'; } catch (e) { $('gitLog').textContent = String(e); }
}
function human(n) { return n < 1024 ? n + ' B' : n < 1048576 ? (n / 1024).toFixed(1) + ' KB' : (n / 1048576).toFixed(1) + ' MB'; }
