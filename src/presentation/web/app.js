'use strict';
/* Solomni 转录中心前端（no-build vanilla JS）。
 * 数据流：REST 动作 → 事件回包渲染；长轮询增量事件（多端同看）；断线重连 + 状态点。
 * 原则：转录即内容（原样渲染）；密钥永不出现在任何请求/界面。
 */

const $ = (s) => document.querySelector(s);
const state = {
  modules: [], providers: [], rejected: [],
  sessions: new Map(),   // sid -> { sid, mode, title, lines: [{cls, who, text}], pending, busy, done, active }
  activeSid: null,
};

/* ---------- API ---------- */
async function api(method, url, body) {
  const res = await fetch(url, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || ('HTTP ' + res.status));
  return data;
}

/* ---------- 状态与侧栏 ---------- */
async function refreshState() {
  const s = await api('GET', '/api/state');
  state.modules = s.modules || [];
  state.providers = s.providers || [];
  state.rejected = s.rejected || [];
  renderSidebar();
}

function renderSidebar() {
  const list = $('#module-list');
  list.innerHTML = '';
  for (const m of state.modules) {
    const el = document.createElement('div');
    el.className = 'module-item';
    el.innerHTML =
      '<div class="mid"></div><div class="mbrief"></div>' +
      '<div class="module-actions">' +
      '<button data-act="direct">直连</button>' +
      '<button data-act="collab">协作</button>' +
      '<button data-act="omni">全能</button>' +
      '</div>';
    el.querySelector('.mid').textContent = m.id;
    el.querySelector('.mbrief').textContent = m.brief;
    el.querySelector('[data-act="direct"]').onclick = () => createSession('direct', m.id);
    el.querySelector('[data-act="collab"]').onclick = () => createSession('collab', m.id);
    el.querySelector('[data-act="omni"]').onclick = () => createSession('omni', m.id);
    list.appendChild(el);
  }
  $('#rejected').textContent = state.rejected.join('\n');
  const pl = $('#provider-list');
  pl.innerHTML = '';
  for (const p of state.providers) {
    const el = document.createElement('div');
    el.className = 'provider-item';
    el.innerHTML = '<span class="pid"></span>' + (p.is_default ? '<span class="pdef">默认</span>' : '') +
      '<span class="pinfo"></span>' +
      '<button data-act="default" title="设为默认">默认</button>' +
      '<button data-act="remove" title="移除">移除</button>';
    el.querySelector('.pid').textContent = p.id;
    el.querySelector('.pinfo').textContent = p.models.join(', ');
    el.querySelector('[data-act="default"]').onclick = async () => { await api('POST', '/api/providers/' + encodeURIComponent(p.id) + '/default'); refreshState(); };
    el.querySelector('[data-act="remove"]').onclick = async () => { await api('POST', '/api/providers/' + encodeURIComponent(p.id) + '/remove'); refreshState(); };
    pl.appendChild(el);
  }
}

$('#provider-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const models = $('#pv-models').value.split(/[,，]/).map(x => x.trim()).filter(Boolean);
  try {
    await api('POST', '/api/providers', {
      id: $('#pv-id').value.trim(), base_url: $('#pv-url').value.trim(),
      api_key: $('#pv-key').value, models,
    });
    $('#pv-key').value = '';
    await refreshState();
  } catch (err) { alert(err.message); }
});

/* ---------- 会话 ---------- */
async function createSession(mode, ids) {
  try {
    const r = await api('POST', '/api/sessions', { mode, ids });
    const title = mode === 'direct' ? '直连·' + ids : mode === 'omni' ? '全能' : '协作·' + (ids === '?' ? '代拟' : ids);
    const s = { sid: r.sid, mode, title, lines: [], pending: null, busy: false, done: false, openEvents: r.events || [] };
    state.sessions.set(r.sid, s);
    for (const ev of s.openEvents) absorb(s, ev);
    s.openEvents = [];
    setActive(r.sid);
    // 协作会话创建后立即进入需求提交。
    if (mode === 'collab') {
      s.awaiting = 'task';
      renderGate(s);
    }
    renderAll();
  } catch (err) { alert(err.message); }
}

function setActive(sid) {
  state.activeSid = sid;
  renderTabs(); renderStream(); renderGate(state.sessions.get(sid));
  $('#input').focus();
}

function activeSession() { return state.sessions.get(state.activeSid); }

/* ---------- 事件吸收：统一把 SessionEvent 变成行 ---------- */
function absorb(s, ev) {
  switch (ev.type) {
    case 'notice': s.lines.push({ cls: 'sys', who: '', text: ev.text }); break;
    case 'transcript': for (const l of ev.lines) s.lines.push(...parseLine(l)); break;
    case 'discussion_done':
      if (ev.over_cap) s.lines.push({ cls: 'sys', who: '', text: '讨论超轮次上限，进入裁决。' });
      break;
    case 'plan': s.lines.push({ cls: 'plan', who: '核心整理', text: ev.text }); break;
    case 'report': s.lines.push({ cls: 'line', who: ev.rework > 0 ? '执行·返工' + ev.rework + ' · ' + ev.id : '执行 · ' + ev.id, text: ev.text }); break;
    case 'review':
      if (!ev.items || ev.items.length === 0) {
        s.lines.push({ cls: 'bad', who: '验收', text: '清单解析失败，原文：\n' + ev.raw });
      } else {
        const txt = ev.items.map(i => '[' + i.status.toUpperCase() + '] ' + i.item + (i.note ? ' —— ' + i.note : '')).join('\n');
        s.lines.push({ cls: 'review', who: '验收清单', text: txt });
      }
      break;
    case 'delivery':
      s.lines.push(ev.ok ? { cls: 'ok', who: '交付', text: '全部通过，交付用户。' }
        : { cls: 'bad', who: '裁决', text: '返工超限仍未通过，交用户裁决。' });
      break;
    case 'ended': s.done = true; break;
  }
}

/* [id:tag] text → 行对象（呈现即上下文：原样收录，标签做样式） */
function parseLine(l) {
  const m = l.match(/^\[([^\]]+):([a-z]+)\]([\s\S]*)$/);
  if (m) {
    const cls = m[2] === 'agree' ? 'ok' : m[2] === 'leave' ? 'sys' : m[2] === 'ask' ? 'plan' : 'line';
    const degraded = l.includes('（信封缺失');
    return [{ cls: degraded ? 'sys' : cls, who: m[1] + ' · ' + m[2] + (degraded ? ' · 信封缺失' : ''), text: m[3].trim() }];
  }
  if (l.startsWith('[用户')) return [{ cls: 'user', who: '用户', text: l.replace(/^\[[^\]]+\]\s*/, '') }];
  if (l.startsWith('[代拟]')) return [{ cls: 'sys', who: '核心代拟', text: l.slice(4) }];
  return [{ cls: 'line', who: '', text: l }];
}

/* ---------- 渲染 ---------- */
function renderAll() { renderTabs(); renderStream(); renderGate(activeSession()); }

function renderTabs() {
  const tabs = $('#tabs');
  tabs.innerHTML = '';
  for (const s of state.sessions.values()) {
    const el = document.createElement('button');
    el.className = 'tab' + (s.sid === state.activeSid ? ' active' : '');
    el.dataset.badge = s.pending && s.sid !== state.activeSid ? '1' : '0';
    el.innerHTML = '<span></span><span class="close">✕</span>';
    el.querySelector('span').textContent = s.title;
    el.onclick = () => setActive(s.sid);
    el.querySelector('.close').onclick = (e) => {
      e.stopPropagation();
      state.sessions.delete(s.sid);
      if (state.activeSid === s.sid) {
        state.activeSid = state.sessions.keys().next().done ? null : state.sessions.keys().next().value;
      }
      renderAll();
    };
    tabs.appendChild(el);
  }
}

function renderStream() {
  const box = $('#stream');
  box.innerHTML = '';
  const s = activeSession();
  if (!s) {
    box.innerHTML = '<div class="empty"><div class="logo">☉</div>' +
      '<div class="tip">从左侧模块公地选择一个模块开始：直连 · 协作 · 全能</div>' +
      '<div class="hint">PC：双栏布局　移动端：左上角 ☰ 打开侧栏</div></div>';
    return;
  }
  for (const l of s.lines) {
    const el = document.createElement('div');
    el.className = 'line ' + l.cls;
    if (l.who) {
      const w = document.createElement('span'); w.className = 'who'; w.textContent = l.who; el.appendChild(w);
    }
    el.appendChild(document.createTextNode(l.text));
    box.appendChild(el);
  }
  if (s.busy) {
    const t = document.createElement('div'); t.className = 'typing'; t.textContent = '成员工作中'; box.appendChild(t);
  }
  box.scrollTop = box.scrollHeight;
}

/* 裁决门：确认名单 / 确认开始 / 请教回答 */
function renderGate(s) {
  const gate = $('#gate');
  gate.innerHTML = '';
  if (!s || s.busy || s.done) return;
  if (s.awaiting === 'task') {
    gate.appendChild(gateCard('请提交本次协作需求：', [
      ['提交', async () => { const v = takeInput(); if (v) await act('task', v); }],
    ]));
  } else if (s.pending) {
    if (s.pending.type === 'confirm_slate') {
      gate.appendChild(gateCard('核心已代拟名单（见转录），是否按此建组？', [
        ['确认建组', () => act('slate', 'yes')],
        ['取消', () => act('slate', 'no')],
      ]));
    } else if (s.pending.type === 'confirm_begin') {
      gate.appendChild(gateCard('名单已定，开始讨论？', [
        ['开始', () => act('begin', 'yes')],
        ['开始（授权小组自裁细节）', () => act('begin', 'yes,allow')],
        ['暂不', () => {}],
      ]));
    } else if (s.pending.type === 'ask') {
      gate.appendChild(gateCard(s.pending.member + ' 请教：' + s.pending.question, [
        ['回答', async () => { const v = takeInput(); if (v !== null) await act('answer', v); }],
      ]));
    }
  }
}

function gateCard(q, btns) {
  const el = document.createElement('div');
  el.className = 'gate-card';
  const qq = document.createElement('div'); qq.className = 'q'; qq.textContent = q; el.appendChild(qq);
  const bs = document.createElement('div'); bs.className = 'btns';
  for (const [label, fn] of btns) {
    const b = document.createElement('button'); b.className = 'btn'; b.textContent = label;
    b.onclick = () => fn().catch(err => alert(err.message));
    bs.appendChild(b);
  }
  el.appendChild(bs);
  return el;
}

/* ---------- 动作 ---------- */
function takeInput() {
  const v = $('#input').value.trim();
  $('#input').value = '';
  autoGrow();
  return v;
}

async function act(action, text) {
  const s = activeSession();
  if (!s || s.busy) return;
  s.busy = true;
  if (text) s.lines.push({ cls: 'user', who: '用户', text });
  renderStream();
  try {
    const r = await api('POST', '/api/sessions/' + s.sid + '/' + action, { text });
    for (const ev of r.events) absorb(s, ev);
    s.awaiting = null;
    await refreshPending(s);
    renderAll();
  } catch (err) {
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    renderAll();
  } finally {
    s.busy = false;
    renderAll();
  }
}

async function refreshPending(s) {
  // 服务端在 Ended 后回收会话：查询报「无此会话」即视为已终结。
  if (s.done) return;
  try {
    const r = await api('POST', '/api/sessions/' + s.sid + '/pending', {});
    s.pending = r.pending || null;
  } catch (err) {
    if (String(err.message).includes('无此会话')) { s.done = true; s.pending = null; }
  }
}

/* ---------- 发送 ---------- */
$('#btn-send').onclick = onSend;
$('#input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); onSend(); }
});
$('#input').addEventListener('input', autoGrow);
function autoGrow() {
  const t = $('#input'); t.style.height = 'auto'; t.style.height = Math.min(t.scrollHeight, 120) + 'px';
}
function onSend() {
  const s = activeSession();
  if (!s) return;
  if (s.awaiting === 'task') { const v = takeInput(); if (v) act('task', v); return; }
  if (s.pending && s.pending.type === 'ask') { const v = takeInput(); if (v) act('answer', v); return; }
  if (s.mode === 'direct' || s.mode === 'omni') { const v = takeInput(); if (v) act('say', v); return; }
  // 协作无挂起时忽略发送（避免打断泵）。
}

/* ---------- 长轮询：增量事件 + 连接状态 ---------- */
let pollSince = 0;
let pollActive = false;
async function pollLoop() {
  if (pollActive) return;
  pollActive = true;
  while (true) {
    try {
      const r = await fetch('/api/events?sid=&since=' + pollSince);
      if (!r.ok) throw new Error('轮询失败 ' + r.status);
      const data = await r.json();
      setConn(true);
      pollSince = data.head ?? pollSince;
      for (const line of data.lines) {
        try {
          const payload = JSON.parse(line);
          const s = state.sessions.get(payload.sid);
          if (!s) continue;
          for (const ev of payload.events) absorb(s, ev);
          s.busy = false;
        } catch { /* 单行损坏不拖垮轮询 */ }
      }
      renderAll();
      // 有会话动作在等回包时，让动作回包自己刷新 pending；轮询只补漏。
    } catch {
      setConn(false);
      await new Promise(res => setTimeout(res, 3000));
    }
  }
}
function setConn(ok) {
  $('#conn-state .dot').className = 'dot ' + (ok ? 'dot-ok' : 'dot-bad');
  $('#conn-text').textContent = ok ? '已连接' : '重连中…';
}

/* ---------- 抽屉（移动端） ---------- */
$('#btn-drawer').onclick = () => { $('#sidebar').classList.add('open'); $('#drawer-mask').classList.add('show'); };
$('#drawer-mask').onclick = () => { $('#sidebar').classList.remove('open'); $('#drawer-mask').classList.remove('show'); };

/* ---------- 启动 ---------- */
refreshState().then(pollLoop).catch(err => alert('初始化失败：' + err.message));