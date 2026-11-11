/* 目的：左侧栏——会话历史树、铅笔菜单、抽屉与宽窄布局。
 * 管：renderSidebar / renderHistory / hydrateHistory / openHistory / deleteHistory / setupSidebar 与抽屉开合。
 * 不管：不拉历史、不改会话内容；打开会话与改配置都走 app 上的入口。
 * 联动：会话读法来自 app/stores/sessions.js；弹层来自 app/ui；刷新走 app.refreshState。 */

import { activePath, generating, historyByParent } from '../../stores/sessions.js';
import { sidebarCopy } from './copy.js';

let g, doc, state, api, ui, app;
let fold, h, icon;
const absorb = (...a) => app.absorb(...a);
const absorbEvents = (...a) => app.absorbEvents(...a);
const eventError = (...a) => app.eventError(...a);
const openConfig = (...a) => app.openConfig(...a);
const openWizard = (...a) => app.openWizard(...a);
const refreshState = (...a) => app.refreshState(...a);
const renderAll = (...a) => app.renderAll(...a);
const setActive = (...a) => app.setActive(...a);

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/// 行尾铅笔菜单：打开 / 编辑配置 / 删除（子会话多一项「打开父会话」，不给删除）。
function historyMenu(h) {
  const busy = generating(h.name);
  /** @type {Array<{label?: string, icon?: string, disabled?: boolean, reason?: string, sep?: boolean, danger?: boolean, onClick?: () => void}>} */
  const items = [
    { label: '打开', icon: 'folder', onClick: () => openHistory(h.name) },
    {
      label: '编辑配置',
      icon: 'pencil',
      disabled: busy,
      reason: busy ? '该会话正在生成中：先「停止」或等它结束，再改配置' : '',
      onClick: () => openConfig(h.name),
    },
  ];
  if (h.parent) items.splice(1, 0, { label: '打开父会话', icon: 'chevron', onClick: () => openHistory(h.parent) });
  if (!h.parent) {
    items.push({ sep: true });
    items.push({ label: '删除', icon: 'close', danger: true, onClick: () => deleteHistory(h.name) });
  }
  return items;
}

function renderHistory() {
  const box = $('#history-list');
  if (!box) return;
  box.innerHTML = '';
  const items = state.history || [];
  if (!items.length) { box.textContent = '（还没有历史会话）'; return; }
  // 活跃路径的祖先自动展开一次（切换会话时）；用户手动折叠后不再被抢回去。
  if (state.autoOpenedFor !== state.activeSid) {
    state.autoOpenedFor = state.activeSid;
    for (const n of activePath()) {
      const h = items.find((x) => x.name === n);
      if (h && h.parent) state.treeOpen.add(h.parent);
    }
  }
  const by = historyByParent();
  const branch = (parent, into) => {
    for (const h of (by.get(parent) || [])) {
      const row = doc.createElement('div');
      row.className = 'history-item' + (h.workspace_missing ? ' bad' : '');
      row.title = h.workspace_missing ? '工作区已不存在：只能查看记录，不能启用' : '点击打开会话';
      row.setAttribute('role', 'button');
      row.tabIndex = 0;
      row.onclick = () => openHistory(h.name);
      row.onkeydown = (e) => {
        if (e.key !== 'Enter' && e.key !== ' ') return;
        e.preventDefault();
        openHistory(h.name);
      };
      const kids = by.get(h.name) || [];
      const open = state.treeOpen.has(h.name);
      if (kids.length) {
        const t = doc.createElement('button');
        t.type = 'button';
        t.className = 'history-toggle' + (open ? ' open' : '');
        t.title = open ? '收起子会话' : '展开子会话';
        t.setAttribute('aria-label', t.title);
        t.appendChild(ui.icon('chevron'));
        t.onclick = (e) => {
          e.stopPropagation();
          if (open) state.treeOpen.delete(h.name); else state.treeOpen.add(h.name);
          renderHistory();
        };
        row.appendChild(t);
      } else {
        const sp = doc.createElement('span'); sp.className = 'history-toggle'; row.appendChild(sp);
      }
      const name = doc.createElement('span');
      name.className = 'hname'; name.textContent = h.name; name.title = h.name;
      const mode = doc.createElement('span'); mode.className = 'hmode';
      // 持久事实（停止 / 关闭）与"这一刻在不在跑"分开：只有确实在跑才标「进行中」，
      // 否则重启前没结束的旧会话会一直冒充在跑。
      const live = state.sessions.get(h.name);
      const running = !!(live && live.running) || !!(state.running && state.running.has(h.name));
      const stateTag = h.run === 'stopped' ? '·已停止'
        : h.run === 'closed' ? '·已关闭'
          : (running && !h.done ? '·进行中 ' : '');
      mode.textContent = stateTag + h.mode +
        (h.tier === 'vm' && h.tier_ready === false ? '·虚拟机档不可用' : '') +
        (h.workspace_missing ? '·工作区不存在' : '');
      const pencil = ui.iconButton({ icon: 'pencil', label: '会话操作', tip: '', cls: 'hpen' });
      pencil.onclick = (e) => { e.stopPropagation(); ui.menu(pencil, historyMenu(h)); };
      row.appendChild(name); row.appendChild(mode); row.appendChild(pencil);
      into.appendChild(row);
      if (kids.length && open) {
        const wrap = doc.createElement('div'); wrap.className = 'history-kids';
        branch(h.name, wrap);
        into.appendChild(wrap);
      }
    }
  };
  branch('', box);
}

/* 记的是虚拟机档、但本机现在承载不了：**不拦打开**（记录是用户的），只主动把原因与出路说清。
 * 为什么必须提示：虚拟机档的 guest 本体尚未接入，此刻选它只会得到一个更差的本机档——
 * 用户有权知道这一点，也有权知道怎么才能用上。 */
function warnIfTierUnavailable(name) {
  const view = (state.history || []).find((h) => h.name === name);
  if (!view || view.tier !== 'vm' || view.tier_ready !== false) return;
  const missing = (view.tier_missing || []).join('；') || '本机不具备虚拟机档的前置条件';
  ui.notice(
    sidebarCopy.vmUnavailable.title,
    '原因：' + missing + '。\n\n' + sidebarCopy.vmUnavailable.kept + '\n' + sidebarCopy.vmUnavailable.enable,
    'warn'
  );
}

/** **历史与实时合流**：后端把盘上转录与"事件台上它之外的尾巴"一次给全，这里只**按序 append**。
 * 合流时的头部序号 = 这条会话的**水位**：水位以下的批次已经在这份合流里给过了，之后收到的一律丢掉。
 * 为什么必须这样：从前前端自己合并「盘上转录」与「事件台重放」两个来源，而行没有共同身份
 * （notice/node_started 这类行根本没有 id），只要补水先于重放完成，整段记录就会重复一遍（真机症状）。 */
async function hydrateHistory(s) {
  if (!s || s.hydrating || s.hydrated) return;
  s.hydrating = true;
  let got = null;
  try {
    got = await api('GET', '/api/history/' + encodeURIComponent(s.sid));
  } catch (err) { eventError(err); }
  if (got) {
    for (const ev of (got.events || [])) absorb(s, ev);
    for (const ev of (got.live || [])) absorb(s, ev);
    if (typeof got.head === 'number') s.floor = got.head;
    // 状态槽只活在本进程里：重启 / 重建后按转录派生的快照补上（已有的实时快照更新，不覆盖）。
    if (got.shell && !s.shell) s.shell = got.shell;
  }
  // 合流期间到达的实时批次先攒着；水位定好后按同一把尺子接上（水位及以上的才接）。
  const buffered = s.buffer || [];
  s.buffer = [];
  for (const item of buffered) {
    if (item && (typeof s.floor !== 'number' || item.seq > s.floor)) {
      absorbEvents(s, item.events || []);
    }
  }
  s.hydrating = false;
  s.hydrated = true;
  renderAll();
}

async function openHistory(name) {
  const existing = state.sessions.get(name);
  if (existing && !existing.readonly) {
    // 事件建出来的会话还没补过盘上转录：先补，再显示（否则只有刷新之后的新行）。
    hydrateHistory(existing);
    setActive(name);
    warnIfTierUnavailable(name);
    if (isNarrow()) closeDrawer();
    return;
  }
  try {
    const r = await api('GET', '/api/history/' + encodeURIComponent(name));
    const s = {
      sid: name, mode: (r.meta && r.meta.mode) || 'single', title: name,
      parent: (r.meta && r.meta.parent) || null,
      lines: [], live: [], pending: null, sending: false,
      running: false, working: null, running_known: false,
      activating: false, continue_action: null,
      context: null, shell: r.shell || null, work: null, workAt: 0, workDirty: false, workError: '',
      permission_profile: '', permission_label: '', thinking: null, context_freshness: false,
      permission_narrowed: false, permission_parent: null,
      done: true, awaiting: null, readonly: true, fold: {}, scroll: {},
    };
    const hist = (state.history || []).find((x) => x.name === name);
    s.workspace = (hist && hist.workspace) || null;
    s.workspace_missing = !!(hist && hist.workspace_missing);
    s.hydrated = true;
    // 合流：盘上转录 + 事件台尾巴一次给全；水位 = 合流时的头部（之后的实时批次照收）。
    if (typeof r.head === 'number') s.floor = r.head;
    state.sessions.set(name, s);
    for (const ev of (r.events || [])) absorb(s, ev);
    for (const ev of (r.live || [])) absorb(s, ev);
    setActive(name);
    renderAll();
    warnIfTierUnavailable(name);
    // 窄屏是抽屉：选完会话就收起来，否则正文被挤成一条窄缝。
    if (isNarrow()) closeDrawer();
  } catch (err) { ui.notice('操作失败', err.message, 'err'); }
}

async function deleteHistory(name) {
  // 外部工作区不归本产品所有：删除只删会话目录，必须先说清“工作区文件保留”。
  const sess = state.sessions.get(name);
  if (sess && sess.workspace) {
    if (!(await ui.confirmBox('删除会话', '删除会话「' + name + '」？记录与版本库将被永久删除；工作区 ' + sess.workspace + ' 里的文件会保留，不会被删除。', '删除'))) return;
    try {
      await api('POST', '/api/history/' + encodeURIComponent(name) + '/delete', {});
      if (state.sessions.has(name)) {
        state.sessions.delete(name);
        if (state.activeSid === name) {
          const it = state.sessions.keys().next();
          state.activeSid = it.done ? null : it.value;
        }
      }
      await refreshState();
      renderAll();
      ui.toast('ok', '已删除会话：' + name);
    } catch (err) { ui.notice('操作失败', err.message, 'err'); }
    return;
  }
  // 删除前如实交代工作区：会被一起删掉的产物先列出来（数量 + 字节），避免"以为只删记录"。
  // 一次 GET /files 拿到同一份清单与用量；拉不到（会话已不存在）就退回一步确认。
  let usage = null;
  try {
    const r = await api('GET', '/api/sessions/' + encodeURIComponent(name) + '/files');
    usage = (r && r.usage) || null;
  } catch (e) { usage = null; }
  if (usage && usage.files > 0) {
    const areas = [];
    if (usage.work && usage.work.files) {
      areas.push('· 共享区 work/：' + usage.work.files + ' 个文件（' + usage.work.bytes + ' 字节）');
    }
    for (const a of Object.keys(usage.agents || {})) {
      const u = usage.agents[a];
      if (u && u.files) areas.push('· 沙箱 ' + a + '/：' + u.files + ' 个文件（' + u.bytes + ' 字节）');
    }
    const text = '会话「' + name + '」的记录将被永久删除，它的工作区里还有 ' + usage.files +
      ' 个文件（共 ' + usage.bytes + ' 字节）：\n' + areas.join('\n') +
      '\n这些文件也会一起删掉（子会话一并删除）。';
    if (!(await ui.confirmBox('删除会话与工作区文件', text, '一起删除'))) return;
  } else if (!(await ui.confirmBox('删除会话', '删除会话「' + name + '」？该会话的记录将被永久删除。', '删除'))) {
    return;
  }
  try {
    await api('POST', '/api/history/' + encodeURIComponent(name) + '/delete', {});
    if (state.sessions.has(name)) {
      state.sessions.delete(name);
      if (state.activeSid === name) {
        const it = state.sessions.keys().next();
        state.activeSid = it.done ? null : it.value;
      }
    }
    await refreshState();
    renderAll();
    ui.toast('ok', '已删除会话：' + name);
  } catch (err) { ui.notice('操作失败', err.message, 'err'); }
}

/* 侧栏：只保留「被拒收模块」的如实提示（模块清单在「新建工作」向导里用）。 */
function renderSidebar() {
  const rejected = $('#rejected');
  if (rejected) rejected.textContent = (state.rejected || []).join('\n');
}

/* ---------- 侧栏：桌面收起为图标栏，移动端是抽屉 ---------- */
const RAIL_KEY = 'solomni.ui.sidebar';

function isNarrow() {
  try { return typeof g.matchMedia === 'function' && g.matchMedia('(max-width: 899px)').matches; }
  catch (e) { return false; }
}

function readRail() {
  try { return g.localStorage.getItem(RAIL_KEY) === 'rail'; } catch (e) { return false; }
}

function saveRail() {
  try { g.localStorage.setItem(RAIL_KEY, state.rail ? 'rail' : 'wide'); } catch (e) { /* 存不了就只留内存态 */ }
}

function applyRail(railed) {
  state.rail = !!railed;
  const sb = $('#sidebar');
  if (sb) sb.className = 'sidebar' + (state.rail && !isNarrow() ? ' rail' : '');
}

function openDrawer() {
  const sb = $('#sidebar'); if (sb) sb.classList.add('open');
  const mask = $('#drawer-mask'); if (mask) mask.classList.add('show');
  const btn = $('#btn-drawer'); if (btn) btn.setAttribute('aria-expanded', 'true');
}

function closeDrawer() {
  const sb = $('#sidebar'); if (sb) sb.classList.remove('open');
  const mask = $('#drawer-mask'); if (mask) mask.classList.remove('show');
  const btn = $('#btn-drawer'); if (btn) btn.setAttribute('aria-expanded', 'false');
}

function toggleSidebar() {
  if (isNarrow()) {
    const sb = $('#sidebar');
    if (sb && sb.classList.contains('open')) closeDrawer();
    else openDrawer();
    return;
  }
  applyRail(!state.rail);
  saveRail();
}

/// 侧栏的图标与底部设置按钮：HTML 只留骨架，图标从组件库 app/ui/icons.js 来。
function setupSidebar() {
  if (typeof ui === 'undefined') return;
  const foot = $('#btn-settings');
  if (foot && !(foot.children || []).length) {
    foot.appendChild(ui.icon('gear'));
    const label = doc.createElement('span');
    label.className = 'sidebar-action-label';
    label.textContent = '设置';
    foot.appendChild(label);
  }
  const rail = $('#sidebar-rail');
  if (rail && !(rail.children || []).length) {
    const expand = () => { applyRail(false); saveRail(); };
    rail.appendChild(ui.iconButton({ icon: 'plus', label: '新建工作', tip: '新建工作', onClick: openWizard }));
    rail.appendChild(ui.iconButton({ icon: 'clock', label: '会话历史', tip: '会话历史', onClick: expand }));
    rail.appendChild(ui.iconButton({ icon: 'gear', label: '设置', tip: '设置', onClick: () => app.openSettingsHub('generate') }));
    rail.appendChild(ui.iconButton({ icon: 'chevron', label: '展开侧栏', tip: '展开侧栏', onClick: expand }));
  }
  const closeBtn = $('#btn-drawer-close');
  if (closeBtn) {
    closeBtn.innerHTML = '';
    closeBtn.appendChild(ui.icon('close'));
    closeBtn.onclick = closeDrawer;
  }
  const drawer = $('#btn-drawer');
  if (drawer) {
    drawer.innerHTML = '';
    drawer.appendChild(ui.icon('panel'));
    drawer.onclick = toggleSidebar;
  }
  applyRail(readRail());
  if (typeof g !== 'undefined' && g.addEventListener) {
    g.addEventListener('resize', () => applyRail(state.rail));
  }
}

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, api, ui, app } = deps);
  ({ fold, h, icon } = ui);
  Object.assign(root, { historyMenu, renderHistory, warnIfTierUnavailable, hydrateHistory, openHistory, deleteHistory, renderSidebar, RAIL_KEY, isNarrow, readRail, saveRail, applyRail, openDrawer, closeDrawer, toggleSidebar, setupSidebar });
}
