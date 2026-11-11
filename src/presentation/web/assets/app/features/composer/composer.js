/* 目的：输入区——发送、停止、斜杠命令、@ 文件引用与自动增高。
 * 管：onSend / stopGeneration / cmdSync / at* 系列与 autoGrow / syncComposer / syncSendButton。
 * 不管：不渲染转录、不改会话状态；发送与停止都走统一动作分发。
 * 联动：会话读法来自 app/stores/sessions.js；动作走 app.act。 */

import { activeSession, isBusy } from '../../stores/sessions.js';

let g, doc, state, api, ui, app;
let h;
const act = (...a) => app.act(...a);
const actionUrl = (...a) => app.actionUrl(...a);
const compactNow = (...a) => app.compactNow(...a);
const loadPathRoots = (...a) => app.loadPathRoots(...a);
const renderReadonly = (...a) => app.renderReadonly(...a);
const syncStatusbar = (...a) => app.syncStatusbar(...a);
const takeInput = (...a) => app.takeInput(...a);

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/* 输入区与上下文继续的显隐：只按后端给的视图（readonly / continue_action / running）渲染。 */
function syncComposer(s) {
  const composer = $('#composer');
  if (composer) {
    if (!s || s.readonly) composer.classList.add('hidden');
    else composer.classList.remove('hidden');
  }
  renderReadonly(s);
  syncStatusbar(s);
  const cont = $('#btn-continue');
  if (!cont) return;
  const c = s && !s.readonly && !isBusy(s) ? s.continue_action : null;
  if (c) cont.classList.remove('hidden');
  else cont.classList.add('hidden');
  if (c) {
    cont.textContent = c.label || '继续';
    cont.title = c.reason || '';
  }
}

function syncSendButton(s) {
  const sendBtn = $('#btn-send');
  if (!sendBtn) return;
  // 忙碌时只留一个「停止」：用户一眼就知道这个会话在跑，而不是拿「发送」去试探。
  const busy = isBusy(s);
  sendBtn.textContent = busy ? '停止' : '发送';
  sendBtn.className = busy ? 'btn btn-danger' : 'btn btn-primary';
  // 只读视图整块隐藏输入区；这里只处理正常会话的发送/停止。
  sendBtn.disabled = false;
  sendBtn.title = '';
  // 「改需求」是**用户的动作**（不是核心推的门）：没有"本次需求"就**根本不渲染**（不是灰着）；
  // 会话正在工作时不可点。
  const up = $('#btn-update-task');
  if (up) {
    const can = !!(s && s.can_update_task);
    up.className = can ? 'btn' : 'btn hidden';
    up.disabled = busy;
  }
}

const atState = { open: false, loading: false, items: [], all: [], active: 0, start: -1, sid: null, mode: '' };

const COMMANDS = [{ name: '/compact', insert: '/compact', desc: '压缩上下文' }];

function atMenuEl() { return $('#at-menu'); }

/* 光标前的 @ 片段：前面必须是行首或空白，且 @ 与光标之间没有空白/另一个 @。 */
function atToken() {
  const t = $('#input');
  const v = String(t.value || '');
  const pos = typeof t.selectionStart === 'number' ? t.selectionStart : v.length;
  const before = v.slice(0, pos);
  const at = before.lastIndexOf('@');
  if (at < 0) return null;
  const frag = before.slice(at + 1);
  if (/[\s@]/.test(frag)) return null;
  if (at > 0 && !/\s/.test(before[at - 1])) return null;
  return { start: at, frag: frag };
}

function atClose() {
  atState.open = false; atState.loading = false; atState.items = []; atState.all = [];
  atState.active = 0; atState.start = -1; atState.sid = null; atState.mode = '';
  const box = atMenuEl();
  if (box) { box.className = 'at-menu hidden'; box.innerHTML = ''; }
}

/* 菜单底部的固定提示行（可发现性）。 */
function atHint(box) {
  const h = doc.createElement('div'); h.className = 'at-hint';
  h.textContent = '↑↓ 选择 · Enter 填入 · Esc 关闭';
  box.appendChild(h);
}

async function atLoad(sid) {
  if (app.filesCache.has(sid)) return app.filesCache.get(sid);
  return atFetch(sid);
}

/* **每次打开 @ 菜单都重拉一次**：文件可能在会外被手工删掉，或被 agent 的工具改过。
 * 前端不引入目录 watcher（本机、跨平台，代价不值）；拉到的结果写回缓存，
 * 供长路径缩写（setActive / loadPathRoots）快速复用。 */
async function atFetch(sid) {
  try {
    const data = await api('GET', '/api/sessions/' + encodeURIComponent(sid) + '/files');
    app.filesCache.set(sid, data);
    return data;
  } catch (e) {
    return null; // 会话已结束 / 无此会话：安静地不弹菜单，不打扰用户
  }
}

/* 菜单条目：共享区 work/ 在前，其后每个 agent 一个分组。 */
/* 需要加双引号的一段：含空白，或含下面这些句读标点（裸引用会在标点处终止，路径会被截断）。
 * 英文句点 . 刻意不在内——后端已把它从终止符去掉，扩展名必须留在路径里。
 * 只影响**插入的文本**；菜单里显示的仍是原始相对路径（不带引号）。
 * 沙箱引用两段各自按需加引号：sandbox:"调研 助手"/"a b.md"。 */
const AT_QUOTE_CHARS = '\\s，。；、！？：,;!?:）)」』】》';

const AT_QUOTE_RE = new RegExp('[' + AT_QUOTE_CHARS + ']');

function atQuotePath(p) {
  return AT_QUOTE_RE.test(p) ? '"' + p + '"' : p;
}

function atItems(data) {
  const out = [];
  for (const f of (data.work || [])) {
    const p = String(f);
    out.push({ group: '共享区 work/', label: 'work/' + p, path: p, insert: '@work:' + atQuotePath(p) });
  }
  for (const a of (data.agents || [])) {
    const name = a && a.name ? String(a.name) : '';
    for (const f of ((a && a.files) || [])) {
      const p = String(f);
      out.push({
        group: '沙箱 ' + name + '/',
        label: name + '/' + p,
        path: p,
        insert: '@sandbox:' + atQuotePath(name) + '/' + atQuotePath(p),
      });
    }
  }
  return out;
}

function atRender() {
  const box = atMenuEl();
  if (!box) return;
  box.className = 'at-menu';
  box.innerHTML = '';
  if (atState.loading) {
    const e = doc.createElement('div'); e.className = 'at-empty'; e.textContent = '载入中…';
    box.appendChild(e);
    atHint(box);
    return;
  }
  if (!atState.items.length) {
    const e = doc.createElement('div'); e.className = 'at-empty'; e.textContent = '（没有匹配的文件）';
    box.appendChild(e);
    atHint(box);
    return;
  }
  let group = null;
  let activeRow = null;
  atState.items.forEach((it, i) => {
    if (it.group !== group) {
      group = it.group;
      const g = doc.createElement('div'); g.className = 'at-group'; g.textContent = group;
      box.appendChild(g);
    }
    const row = doc.createElement('div');
    const on = i === atState.active;
    row.className = 'at-item' + (on ? ' active' : '');
    row.textContent = it.label;
    row.onclick = () => atInsert(it);
    if (on) activeRow = row;
    box.appendChild(row);
  });
  atHint(box);
  // 长清单里把活动项滚进视野（桩 DOM 没这个方法就跳过）
  if (activeRow && typeof activeRow.scrollIntoView === 'function') {
    try { activeRow.scrollIntoView({ block: 'nearest' }); } catch (e) { /* 忽略 */ }
  }
}

/* 过滤：匹配相对路径子串（大小写不敏感）。 */
function atOpen(frag) {
  const q = String(frag || '').toLowerCase();
  atState.items = atState.all.filter((it) => !q || it.path.toLowerCase().indexOf(q) >= 0 || it.label.toLowerCase().indexOf(q) >= 0);
  if (atState.active >= atState.items.length) atState.active = 0;
  atState.open = true;
  atRender();
}

/* 插入：普通引用替换 @ 片段；命令直接成为整段输入（已经是它时一次 Enter 就执行）。 */
function atInsert(it) {
  const t = $('#input');
  if (it.command) {
    const already = String(t.value || '').trim() === it.insert;
    t.value = it.insert;
    atClose();
    autoGrow();
    if (typeof t.focus === 'function') t.focus();
    if (already) onSend();
    return;
  }
  const v = String(t.value || '');
  const pos = typeof t.selectionStart === 'number' ? t.selectionStart : v.length;
  const start = atState.start >= 0 ? atState.start : pos;
  const text = it.insert + ' ';
  t.value = v.slice(0, start) + text + v.slice(pos);
  const caret = start + text.length;
  if (typeof t.setSelectionRange === 'function') { try { t.setSelectionRange(caret, caret); } catch (e) { /* 桩 DOM 没有就算了 */ } }
  atClose();
  autoGrow();
  if (typeof t.focus === 'function') t.focus();
}

/* ↑/↓/Enter/Esc 只在菜单打开时被菜单消费；返回 false 就交回发送/换行。
 * 菜单开着时 Enter **永远不发送**：载入中或没有匹配项就什么都不做（什么都不插入），
 * 有选中项才填入引用。Shift+Enter 不受影响（交回换行）。 */
function atKey(e) {
  if (!atState.open) return false;
  if (e.key === 'Escape') { atClose(); return true; }
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    if (!atState.loading && atState.items.length) atInsert(atState.items[atState.active]);
    return true;
  }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    const n = atState.items.length;
    if (!atState.loading && n) {
      atState.active = e.key === 'ArrowDown' ? (atState.active + 1) % n : (atState.active - 1 + n) % n;
      atRender();
    }
    return true;
  }
  return false;
}

/* 输入框以 `/` 开头的命令菜单：只在整段输入仍是单个命令片段时弹，匹配不到就交回普通发送。 */
function cmdSync() {
  const t = $('#input');
  const v = String(t.value || '');
  const pos = typeof t.selectionStart === 'number' ? t.selectionStart : v.length;
  const before = v.slice(0, pos);
  if (!before.startsWith('/')) {
    if (atState.mode === 'command') atClose();
    return false;
  }
  const frag = before.slice(1);
  if (/\s/.test(frag)) {
    if (atState.mode === 'command') atClose();
    return false;
  }
  const s = activeSession();
  if (!s || isBusy(s) || s.readonly) {
    if (atState.mode === 'command') atClose();
    return false;
  }
  const q = frag.toLowerCase();
  const items = COMMANDS.filter((c) => !q || c.name.slice(1).toLowerCase().startsWith(q));
  if (!items.length) {
    if (atState.mode === 'command') atClose();
    return false;
  }
  const keep = Number((state.settings && state.settings.compact_keep_percent) || 0);
  const desc = keep > 0 ? '压缩上下文（保留最近 ' + keep + '% 窗口）' : '压缩上下文（只留摘要）';
  atState.mode = 'command';
  atState.open = true;
  atState.loading = false;
  atState.sid = s.sid;
  atState.start = 0;
  atState.items = items.map((c) => ({
    group: '命令',
    label: c.name + ' — ' + (c.name === '/compact' ? desc : c.desc),
    insert: c.insert,
    command: true,
  }));
  atState.all = atState.items.slice();
  atState.active = 0;
  atRender();
  return true;
}

async function atOnInput() {
  if (cmdSync()) return;
  const tok = atToken();
  const s = activeSession();
  // 忙碌（生成中）仍拒绝，避免与「停止」抢键盘；readonly（历史只读会话）允许——插入文本无害。
  if (!tok || !s || isBusy(s)) { atClose(); return; }
  // 同步先开菜单：/files 还没有回来，↑↓/Enter 也必须已经被菜单接管，
  // 否则这期间按 Enter 会把消息直接发出去。
  atState.open = true;
  atState.loading = true;
  atState.items = []; atState.all = []; atState.active = 0;
  atState.sid = s.sid; atState.start = tok.start;
  atRender();
  const data = await atFetch(s.sid); // 每次打开都重拉：不跨打开缓存（会外删除立刻可见）
  if (atState.sid !== s.sid || !atState.open) return; // 拉取期间切了会话 / 菜单已关
  if (!data) { atClose(); return; }                   // 拉不到：安静收起，之后 Enter 恢复为正常发送
  const tok2 = atToken(); // 拉取期间内容可能又变了，重新确认
  if (!tok2) { atClose(); return; }
  atState.loading = false;
  atState.start = tok2.start; atState.all = atItems(data);
  atOpen(tok2.frag);
  loadPathRoots(s.sid); // 顺手把路径根应用上（同一份缓存，命中即立刻生效）
}

function autoGrow() {
  const t = $('#input'); t.style.height = 'auto'; t.style.height = Math.min(t.scrollHeight, 200) + 'px';
}

/* 停止生成：只置位服务端的中止开关；in-flight 的 say/continue 会立刻收尾返回。 */
async function stopGeneration() {
  const s = activeSession();
  if (!s || !isBusy(s)) return;
  try {
    await api('POST', actionUrl('control_session'), { session_id: s.sid, action: 'stop' });
    ui.toast('ok', '已请求停止');
  } catch (e) {
    ui.toast('err', '停止失败：' + e.message);
  }
}

function onSend() {
  const s = activeSession();
  if (!s) return;
  if (s.readonly) { ui.notice('历史回放为只读', '点「启用会话」后才能发送消息。', 'info'); return; }
  if (isBusy(s)) { stopGeneration(); return; } // 生成中：同一个键变成「停止」
  // 整段输入恰好是命令：`/compact` 走动作，不把命令本身发给模型。
  if (String($('#input').value || '').trim() === '/compact') {
    takeInput();
    compactNow(s);
    return;
  }
  if (s.awaiting === 'task') { const v = takeInput(); if (v) act('task', v); return; }
  // 裁决门挂着时不从这里发：那是卡片上的选项（要补充的话写在卡片里的附言框）。
  if (s.pending && s.pending.type === 'decision_card') {
    ui.notice('请按卡片作答', '这一关请点卡片上的选项；要补充的话写在卡片里的附言框。', 'info');
    return;
  }
  if (s.mode !== 'collab') { const v = takeInput(); if (v) act('say', v); return; } // 单 agent 形态可以自由发言
  // 协作无挂起时忽略发送（避免打断泵）。
}

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, api, ui, app } = deps);
  ({ h } = ui);
  Object.assign(root, { syncComposer, syncSendButton, atState, COMMANDS, atMenuEl, atToken, atClose, atHint, atLoad, atFetch, AT_QUOTE_CHARS, AT_QUOTE_RE, atQuotePath, atItems, atRender, atOpen, atInsert, atKey, cmdSync, atOnInput, autoGrow, stopGeneration, onSend });
}
