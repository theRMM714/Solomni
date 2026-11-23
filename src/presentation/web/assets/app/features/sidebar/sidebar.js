/* 目的：左侧栏——会话历史树、铅笔菜单、抽屉与宽窄布局。
 * 管：renderSidebar / renderHistory / hydrateHistory / openHistory / deleteHistory / setupSidebar 与抽屉开合。
 * 不管：不拉历史、不改会话内容；打开会话与改配置都走 app 上的入口。
 * 联动：会话读法来自 app/stores/sessions.js；弹层来自 app/ui；刷新走 app.refreshState。 */

import { activePath, generating } from '../../stores/sessions.js';
import { sidebarCopy } from './copy.js';

let g, doc, state, api, ui, app;
let fold, h, icon;
const absorb = (...a) => app.absorb(...a);
const actionUrl = (...a) => app.actionUrl(...a);
const absorbEvents = (...a) => app.absorbEvents(...a);
const eventError = (...a) => app.eventError(...a);
const openConfig = (...a) => app.openConfig(...a);
const openWizard = (...a) => app.openWizard(...a);
const refreshState = (...a) => app.refreshState(...a);
const renderAll = (...a) => app.renderAll(...a);
const setActive = (...a) => app.setActive(...a);

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/// 行尾铅笔菜单：打开 / 编辑配置 / 停止会话 / 归档 / 删除
/// （子会话多一项「打开父会话」，不给删除；已归档的只给「恢复归档」与删除）。
/// 已停止的不再给「停止会话」：恢复入口在只读条的「启用会话」，避免同一个动作出现两处。
function historyMenu(h) {
  const busy = generating(h.name);
  const stopped = h.run === 'stopped';
  const archived = h.run === 'archived';
  /** @type {Array<{label?: string, icon?: string, disabled?: boolean, reason?: string, sep?: boolean, danger?: boolean, onClick?: () => void}>} */
  const items = [
    { label: '打开', icon: 'folder', onClick: () => openHistory(h.name) },
    {
      label: '编辑配置',
      icon: 'pencil',
      disabled: busy || archived,
      reason: archived ? '这条会话已归档：先「恢复归档」再改配置'
        : busy ? '该会话正在生成中：先「中断」或等它结束，再改配置' : '',
      onClick: () => openConfig(h.name),
    },
  ];
  if (h.parent) items.splice(1, 0, { label: '打开父会话', icon: 'chevron', onClick: () => openHistory(h.parent) });
  if (archived) {
    items.push({ label: '恢复归档', icon: 'restore', onClick: () => restoreHistory(h.name) });
  } else {
    // 停止会话 = 冻结整棵子树（派发与唤醒都拒），与输入键的「中断」不是一回事。
    if (!stopped) items.push({ label: '停止会话', icon: 'pause', onClick: () => stopHistory(h.name) });
    items.push({ label: '归档', icon: 'archive', onClick: () => archiveHistory(h.name) });
  }
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
  // 归档会话从主树收起（统一放到下面的「已归档」段），主树只走还活着的会话。
  const by = new Map();
  for (const h of items) {
    if (h.run === 'archived') continue;
    const p = h.parent || '';
    if (!by.has(p)) by.set(p, []);
    by.get(p).push(h);
  }
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
      // 持久事实（停止）与"这一刻在不在跑"分开：只有确实在跑才标「进行中」，
      // 否则重启前没结束的旧会话会一直冒充在跑。
      const live = state.sessions.get(h.name);
      const running = !!(live && live.running) || !!(state.running && state.running.has(h.name));
      const stateTag = h.run === 'stopped' ? '·已停止' : (running && !h.done ? '·进行中 ' : '');
      mode.textContent = stateTag + app.typeLabel(h.kind) +
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
  renderArchivedSection(box, items.filter((h) => h.run === 'archived'));
}

/* 「已归档」段：默认展开，点标题折叠。**按主会话名归类**——归档的子会话要让人一眼看出
 * 它属于哪个主会话，而不是散成一堆同名子会话（用户明确要求）。 */
function renderArchivedSection(box, items) {
  if (!items.length) return;
  const open = state.archiveOpen !== false;
  const head = doc.createElement('button');
  head.type = 'button';
  head.className = 'history-archived-head';
  head.textContent = (open ? '▾ ' : '▸ ') + '已归档（' + items.length + '）';
  head.setAttribute('aria-expanded', open ? 'true' : 'false');
  head.onclick = () => { state.archiveOpen = !open; renderHistory(); };
  box.appendChild(head);
  if (!open) return;
  const groups = new Map();
  for (const it of items) {
    const root = it.parent || it.name;
    if (!groups.has(root)) groups.set(root, []);
    groups.get(root).push(it);
  }
  for (const [root, rows] of groups) {
    if (rows.some((r) => r.parent)) {
      const gt = doc.createElement('div');
      gt.className = 'history-archived-group';
      gt.textContent = '主会话：' + root;
      gt.title = '这些子会话归档自「' + root + '」';
      box.appendChild(gt);
    }
    for (const it of rows) box.appendChild(archivedRow(it));
  }
}

/// 归档行的最小版式：名字 + ·已归档 + 铅笔菜单；点开是只读视图，恢复从菜单或只读条走。
function archivedRow(it) {
  const row = doc.createElement('div');
  row.className = 'history-item archived';
  row.title = '已归档：点开只读查看，可「恢复归档」';
  row.setAttribute('role', 'button');
  row.tabIndex = 0;
  row.onclick = () => openHistory(it.name);
  row.onkeydown = (e) => {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    e.preventDefault();
    openHistory(it.name);
  };
  const sp = doc.createElement('span'); sp.className = 'history-toggle'; row.appendChild(sp);
  const name = doc.createElement('span');
  name.className = 'hname'; name.textContent = it.name; name.title = it.name;
  const mode = doc.createElement('span'); mode.className = 'hmode';
  mode.textContent = '·已归档 ' + app.typeLabel(it.kind);
  const pencil = ui.iconButton({ icon: 'pencil', label: '会话操作', tip: '', cls: 'hpen' });
  pencil.onclick = (e) => { e.stopPropagation(); ui.menu(pencil, historyMenu(it)); };
  row.appendChild(name); row.appendChild(mode); row.appendChild(pencil);
  return row;
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
      sid: name, kind: (r.meta && r.meta.kind) || '', title: name,
      parent: (r.meta && r.meta.parent) || null,
      lines: [], live: [], pending: null, sending: false,
      running: false, working: null, running_known: false,
      activating: false, continue_action: null,
      context: null, shell: r.shell || null, work: null, workAt: 0, workDirty: false, workError: '',
      permission_profile: '', permission_label: '', thinking: null, context_freshness: false,
      usage_totals: null, freshness_keep_percent: 80, freshness_stats: null,
      permission_narrowed: false, permission_parent: null,
      done: true, awaiting: null, readonly: true, fold: {}, scroll: {},
    };
    const hist = (state.history || []).find((x) => x.name === name);
    s.workspace = (hist && hist.workspace) || null;
    s.workspace_missing = !!(hist && hist.workspace_missing);
    s.run = (hist && hist.run) || 'active';
    // 历史回放也显示累计花费：累计量、开关与保底都是落盘事实（meta），不是运行态。
    const meta = r.meta || {};
    s.usage_totals = meta.usage_totals || null;
    s.context_freshness = !!meta.context_freshness;
    s.freshness_keep_percent = meta.freshness_keep_percent == null ? 80 : Number(meta.freshness_keep_percent);
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

/// 停止会话：先二次确认——停掉后这条会话（含子会话）不再接受派发与唤醒，要「启用会话」才回到工作。
async function stopHistory(name) {
  const ok = await ui.confirmBox(
    '停止会话',
    '停止「' + name + '」（含它的子会话）？停止后不再接受派发与唤醒，输入区会收起；之后点「启用会话」可以接着用。',
    '停止会话'
  );
  if (!ok) return;
  try {
    await api('POST', actionUrl('control_session'), { session_id: name, action: 'stop' });
    await refreshState();
    renderAll();
    ui.toast('ok', '已停止会话：' + name);
  } catch (err) { ui.notice('操作失败', err.message, 'err'); }
}
/// 归档：整棵子树从主列表收起，记录都在；不删数据，随时可「恢复归档」。
async function archiveHistory(name) { await app.archiveSession(name); }

/// 恢复归档：翻回 active，不唤醒；要能对话再点「启用会话」。
async function restoreHistory(name) { await app.restoreArchive(name); }

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
