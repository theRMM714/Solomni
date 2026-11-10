/* 目的：会话标签条——主 → 子 → 子子逐层展开，标出忙态与待办。
 * 管：tabLabel / tabBusy / tabPending / tabPendingDeep / closeView / openPath / renderTabs。
 * 不管：不加载会话、不改运行态；打开与关闭都走 app 上的入口。
 * 联动：父子关系与忙态来自 app/stores/sessions.js。 */

let g, doc, state, stores, api, ui, app;
let el;
const activePath = (...a) => app.activePath(...a);
const knownChildren = (...a) => app.knownChildren(...a);
const openHistory = (...a) => app.openHistory(...a);
const renderAll = (...a) => app.renderAll(...a);
const sessionParent = (...a) => app.sessionParent(...a);
const setActive = (...a) => app.setActive(...a);
const snapshotRunning = (...a) => app.snapshotRunning(...a);

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

function tabLabel(sid) {
  const meta = state.nodeBySid.get(sid);
  if (meta && meta.node) return meta.node + (meta.assignee ? ' · ' + meta.assignee : '');
  const parent = sessionParent(sid);
  if (parent && sid.indexOf(parent + '--') === 0) return sid.slice(parent.length + 2);
  const s = state.sessions.get(sid);
  return (s && s.title) || sid;
}

function tabBusy(sid) {
  const s = state.sessions.get(sid);
  if (s && s.running_known) return !!s.running;
  return snapshotRunning(sid);
}

function tabPending(sid) {
  const s = state.sessions.get(sid);
  if (s && s.pending) return true;
  const v = state.views && state.views.get(sid);
  return !!(v && v.pending);
}

/// 裁决角标向上聚合：子会话在等用户，父标签也亮一个小点。
function tabPendingDeep(sid, seen) {
  const mark = seen || new Set();
  if (mark.has(sid)) return false;
  mark.add(sid);
  if (tabPending(sid)) return true;
  return knownChildren(sid).some((k) => tabPendingDeep(k, mark));
}

/// 关闭一个标签视图：连同它的后代视图一起关（只关视图，不删会话数据）。
function closeView(sid) {
  const doomed = new Set([sid]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const s of [...state.sessions.values()]) {
      const p = s.parent || sessionParent(s.sid);
      if (p && doomed.has(p) && !doomed.has(s.sid)) { doomed.add(s.sid); grew = true; }
    }
  }
  for (const n of doomed) state.sessions.delete(n);
  if (doomed.has(state.activeSid)) {
    let cur = sessionParent(sid);
    while (cur && !state.sessions.has(cur)) cur = sessionParent(cur);
    state.activeSid = cur || (state.sessions.keys().next().done ? null : state.sessions.keys().next().value);
  }
  ui.toast('ok', '已关闭标签（会话仍在）');
  renderAll();
}

/// 打开一条会话并补齐它的祖先视图：标签梯级才有"主 → 子 → 子子"的路径可站。
async function openPath(sid) {
  if (!sid) return;
  const chain = [];
  const seen = new Set();
  let cur = sid;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    chain.unshift(cur);
    cur = sessionParent(cur);
  }
  for (const name of chain) {
    if (!state.sessions.has(name)) await openHistory(name);
  }
  if (state.sessions.has(sid)) setActive(sid);
}

function renderTabs() {
  const tabs = $('#tabs');
  if (!tabs) return;
  tabs.innerHTML = '';
  const path = activePath().reverse(); // 根 → … → 活跃
  const roots = [...state.sessions.values()]
    .filter((s) => !(s.parent || sessionParent(s.sid)))
    .map((s) => s.sid);
  const rootNames = roots.length ? roots : (path[0] ? [path[0]] : []);
  const rows = [];
  if (rootNames.length) rows.push({ active: path[0] || null, names: rootNames, depth: 0 });
  for (let i = 0; i < path.length; i += 1) {
    const kids = knownChildren(path[i]);
    if (!kids.length) break;
    rows.push({ active: path[i + 1] || null, names: kids, depth: i + 1 });
  }
  for (const row of rows) {
    const rowEl = doc.createElement('div');
    rowEl.className = 'tab-row' + (row.depth ? ' sub' : ' root');
    for (const name of row.names) {
      const opened = state.sessions.has(name);
      const el = doc.createElement('button');
      el.type = 'button';
      el.className = 'tab' + (name === state.activeSid ? ' active' : '') + (opened ? '' : ' ghost');
      el.title = name;
      el.setAttribute('aria-selected', name === state.activeSid ? 'true' : 'false');
      el.dataset.badge = tabPendingDeep(name) && name !== state.activeSid ? '1' : '0';
      const label = doc.createElement('span');
      label.textContent = tabLabel(name);
      el.appendChild(label);
      if (tabBusy(name)) {
        const dot = doc.createElement('span');
        dot.className = 'dot dot-ok tab-dot';
        el.appendChild(dot);
      }
      if (opened) {
        const close = doc.createElement('span');
        close.className = 'close';
        close.textContent = '✕';
        close.onclick = (e) => { e.stopPropagation(); closeView(name); };
        el.appendChild(close);
      }
      el.onclick = () => { if (state.sessions.has(name)) setActive(name); else openPath(name); };
      rowEl.appendChild(el);
    }
    tabs.appendChild(rowEl);
  }
}

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, stores, api, ui, app } = deps);
  ({ el } = ui);
  Object.assign(root, { tabLabel, tabBusy, tabPending, tabPendingDeep, closeView, openPath, renderTabs });
}
