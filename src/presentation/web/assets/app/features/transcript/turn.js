/* 目的：定稿的「一轮」气泡（正文 / 思维链 / 工具组 / 节点卡）与只追加的增量渲染。
 * 管：buildTurn / nodeCard / appendFinalRow / appendFinalLine / renderDone，以及 DOM 归属（boxes.owner 等）。
 * 不管：不吸收事件、不做整帧策略、不处理裁决卡（见 ./events.js 与 ./cards.js）。
 * 联动：行部件来自 ./parts.js，纯映射来自 ./model.js。 */

import { boxes } from './dom.js';
import { turnWorthy, turnKey, turnText, turnReasoning } from './model.js';
import { lineBlock, appendBody, foldBind, toolDetail, toolCard, reasoningBlock, rewindButton, flushScroll, sessionNote } from './parts.js';

let g, doc, state, stores, api, ui, app;
const withdrawAgree = (...a) => app.withdrawAgree(...a);
const restoreMark = (...a) => app.restoreMark(...a);

/* 转录 DOM：定稿容器 + 流式容器（都挂在 #stream 下）。
 * 定稿容器只在"整帧重建"时重建；流式容器在 delta / tool_call 路径上只做**增量 append**，
 * 已有节点绝不重建——<details> 的开关、<pre> 的滚动条、正在拖的滚动位置都不会被打断。 */
/* 定稿渲染：按 reply 归成「一轮」，只追加新行。
 * 一轮 = 首次模型回复的 id；同 reply 的正文 / 思维链 / 工具行合成一个气泡。
 * 增量：只处理 s.lines 里还没渲染过的尾部；同一 reply 的后续行只重建这一轮（折叠状态按 key 恢复）。
 * 整帧重建只发生在：切会话、重置/回档、思维链开关、会话说明变化、渲染计数倒退。 */

/// 一轮气泡：身份 / 思维链 / 正文 / 工具组；思维链与工具组各自折叠，整轮可折叠。
function buildTurn(group, s) {
  const parts = group.parts;
  const tools = parts.filter((p) => p.cls === 'tool');
  const msgs = parts.filter((p) => p.cls !== 'tool');
  const head = msgs[0] || parts[0];
  const text = turnText(parts);
  const reasoning = turnReasoning(parts);
  if (!reasoning && !text.trim() && !tools.length) return null;
  const key = turnKey(group);
  const el = doc.createElement('article');
  el.className = 'turn' + (head.cls === 'ok' ? ' ok' : '');
  el.dataset.reply = String(group.reply || '');

  const headEl = doc.createElement('div');
  headEl.className = 'turn-head';
  const who = doc.createElement('span');
  who.className = 'turn-who';
  who.textContent = head.who || head.speaker || '';
  headEl.appendChild(who);
  if (tools.length) {
    const failed = tools.filter((p) => p.tool && !p.tool.ok).length;
    const cnt = doc.createElement('span');
    cnt.className = 'turn-count';
    cnt.textContent = '工具 ' + tools.length + (failed ? ' · ' + failed + ' 失败' : '');
    headEl.appendChild(cnt);
  }
  const preview = doc.createElement('span');
  preview.className = 'turn-preview';
  preview.textContent = text.replace(/\s+/g, ' ').slice(0, 64);
  headEl.appendChild(preview);
  const toggle = ui.iconButton({ icon: 'chevron', label: '折叠这一轮', tip: '', cls: 'turn-toggle' });
  headEl.appendChild(toggle);
  el.appendChild(headEl);

  const body = doc.createElement('div');
  body.className = 'turn-body';
  if (reasoning && state.settings.show_reasoning) {
    body.appendChild(reasoningBlock(reasoning, s, key + ':cot'));
  }
  if (text.trim()) {
    const t = doc.createElement('div');
    t.className = 'turn-text';
    appendBody(t, head.cls, text);
    body.appendChild(t);
  }
  // 原文是**回复级事实**：一条回复只显示一次；原生模式与可见正文相同，不重复。
  const raw = parts.map((p) => p.raw).find((x) => typeof x === 'string' && x.trim()) || '';
  if (raw.trim() && raw.trim() !== text.trim()) {
    body.appendChild(toolDetail('原文', raw, s, key + ':raw', true));
  }
  if (tools.length) {
    const groupEl = doc.createElement('details');
    groupEl.className = 'fold turn-tools';
    foldBind(s, key + ':tools', groupEl);
    const failed = tools.filter((p) => p.tool && !p.tool.ok).length;
    const sum = doc.createElement('summary');
    sum.textContent = '工具调用 · ' + tools.length + (failed ? '（' + failed + ' 失败）' : '');
    groupEl.appendChild(sum);
    const list = doc.createElement('div');
    list.className = 'tool-list';
    for (const p of tools) list.appendChild(toolCard(p.tool, s, 'T' + p.id, ''));
    groupEl.appendChild(list);
    body.appendChild(groupEl);
  }
  el.appendChild(body);

  const foot = doc.createElement('div');
  foot.className = 'turn-foot';
  if (typeof parts[0].id === 'number') {
    const rb = rewindButton(parts[0].id);
    rb.title = '回档到这一轮之前：留档（可恢复）或删除（不可恢复）';
    foot.appendChild(rb);
  }
  if (head.verb === 'agree' && head.speaker) {
    const w = ui.iconButton({ icon: 'close', label: '撤回同意', tip: '', cls: 'line-act' });
    w.title = '让该 agent 本轮不再算同意';
    w.onclick = () => app.withdrawAgree(head.speaker);
    foot.appendChild(w);
  }
  if (foot.children.length) el.appendChild(foot);

  const openKey = key + ':turn';
  const closed = !!(s.fold && s.fold[openKey] === false);
  if (closed) {
    body.classList.add('hidden');
    el.classList.add('collapsed');
  }
  toggle.setAttribute('aria-expanded', String(!closed));
  toggle.onclick = () => {
    const closeNow = !body.classList.contains('hidden');
    body.classList[closeNow ? 'add' : 'remove']('hidden');
    el.classList[closeNow ? 'add' : 'remove']('collapsed');
    s.fold = s.fold || {};
    s.fold[openKey] = !closeNow;
    toggle.setAttribute('aria-expanded', String(!closeNow));
  };
  return { node: el, reply: group.reply, parts, key };
}

/// 节点卡：执行进度，不是转录内容。状态 / 执行人 / 返工轮次 / 回报 / 进入子会话。
function nodeCard(l, s) {
  const n = l.node || {};
  const el = doc.createElement('article');
  el.className = 'node-card ' + (l.state === 'done' ? 'done' : 'started');
  const head = doc.createElement('div');
  head.className = 'node-head';
  const dot = doc.createElement('span');
  dot.className = 'node-dot';
  head.appendChild(dot);
  const title = doc.createElement('span');
  title.className = 'node-title';
  title.textContent = '节点 · ' + (n.id || '（未命名）');
  head.appendChild(title);
  const badge = doc.createElement('span');
  badge.className = 'node-badge';
  badge.textContent = l.state === 'done'
    ? '完成' + (l.rework ? ' · 第 ' + l.rework + ' 轮返工' : '')
    : '进行中';
  head.appendChild(badge);
  if (n.sid) {
    const go = doc.createElement('button');
    go.type = 'button';
    go.className = 'node-open';
    go.title = '进入这个节点的子会话';
    go.appendChild(ui.icon('folder'));
    const label = doc.createElement('span');
    label.textContent = '子会话';
    go.appendChild(label);
    go.onclick = () => app.openPath(n.sid);
    head.appendChild(go);
  }
  el.appendChild(head);
  if (n.assignee) {
    const sub = doc.createElement('div');
    sub.className = 'node-sub';
    sub.textContent = '执行：' + n.assignee;
    el.appendChild(sub);
  }
  if (l.report) {
    const prev = doc.createElement('div');
    prev.className = 'node-preview';
    prev.textContent = String(l.report).split('\n')[0].slice(0, 96);
    el.appendChild(prev);
    const d = doc.createElement('details');
    d.className = 'fold node-report';
    foldBind(s, 'N' + (n.id || ''), d);
    const sum = doc.createElement('summary');
    sum.textContent = '回报全文';
    d.appendChild(sum);
    const pre = doc.createElement('pre');
    pre.className = 'node-report-text';
    pre.textContent = l.report;
    d.appendChild(pre);
    el.appendChild(d);
  }
  return el;
}

function appendFinalRow(l, s) {
  if (l.cls === 'node') { boxes.done.appendChild(nodeCard(l, s)); return; }
  const bodyless = !String(l.text == null ? '' : l.text).trim();
  if (bodyless && !l.reasoning) return;
  const built = lineBlock(l, s, 'L' + l.id, false);
  if (typeof l.id === 'number') built.node.appendChild(rewindButton(l.id));
  if (typeof l.mark === 'number') {
    const rb = ui.iconButton({ icon: 'undo', label: '恢复留档', tip: '', cls: 'line-act' });
    rb.title = '恢复到这次留档之前（标记及其后的内容会被删除）';
    rb.onclick = () => app.restoreMark(l.mark);
    built.node.appendChild(rb);
  }
  boxes.done.appendChild(built.node);
}

function appendFinalLine(l, s) {
  if (!turnWorthy(l)) {
    boxes.doneState.last = null;
    appendFinalRow(l, s);
    return;
  }
  const reply = l.reply || 0;
  const last = boxes.doneState.last;
  if (last && reply && last.reply === reply) {
    // 同一次回复的后续行（典型是工具行）：只重建这一轮，节点替换，折叠状态按 key 恢复。
    const parts = last.parts.concat([l]);
    const fresh = buildTurn({ kind: 'turn', reply, parts, id: parts[0].id }, s);
    if (!fresh) return;
    if (last.node.parentNode && last.node.parentNode.replaceChild) {
      last.node.parentNode.replaceChild(fresh.node, last.node);
    } else {
      boxes.done.removeChild(last.node);
      boxes.done.appendChild(fresh.node);
    }
    boxes.doneState.last = fresh;
    return;
  }
  const turn = buildTurn({ kind: 'turn', reply, parts: [l], id: l.id }, s);
  if (!turn) return;
  boxes.done.appendChild(turn.node);
  boxes.doneState.last = turn;
}

function renderDone(s, full) {
  if (!boxes.done) return;
  const note = sessionNote(s);
  const noteText = note ? note.textContent : '';
  const show = !!state.settings.show_reasoning;
  const bad = full || !!s.forceFull || boxes.doneState.owner !== s.sid || boxes.doneState.count > s.lines.length
    || boxes.doneState.show !== show || boxes.doneState.note !== noteText;
  if (bad) {
    s.forceFull = false;
    boxes.done.innerHTML = '';
    boxes.doneState = { owner: s.sid, count: 0, show, note: noteText, last: null };
    if (note) boxes.done.appendChild(note);
    for (const l of s.lines) appendFinalLine(l, s);
    boxes.doneState.count = s.lines.length;
  } else if (boxes.doneState.count < s.lines.length) {
    for (const l of s.lines.slice(boxes.doneState.count)) appendFinalLine(l, s);
    boxes.doneState.count = s.lines.length;
  }
  flushScroll(s);
}

export { buildTurn, nodeCard, appendFinalRow, appendFinalLine, renderDone };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, stores, api, ui, app } = deps);
  Object.assign(root, { buildTurn, nodeCard, appendFinalRow, appendFinalLine, renderDone });
}
