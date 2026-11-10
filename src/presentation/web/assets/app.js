'use strict';
/* Solomni 转录中心前端（no-build vanilla JS）。
 * 数据流：REST 动作 → 事件回包渲染；长轮询增量事件（多端同看）；断线重连 + 状态点。
 * 原则：转录即内容（原样渲染）；密钥永不出现在任何请求/界面。
 * 版图：侧栏（新建工作 / 会话历史 / 设置二级菜单，含 agent 管理）；主区（会话 tab + 转录 + 裁决门 + 输入区 + 弹层）。
 * 编排：agent = 一个 AI + N 份能力（模块）+ 一个模型。形态只有两种：
 *       single = 1 个 agent（勾 1 个模块即"直连式"，勾多个即"组合式"）；collab = N 个 agent（各自独立沙箱）。
 * 发言主体只有 agent：转录行的说话人、成员列表、撤回同意、失败归属一律以 **agent 实例名** 为准；
 *       "模块" 只是 agent 的能力包（只出现在选能力的地方）。
 */

const $ = (s) => doc.querySelector(s);
/* 状态、HTTP 与事件流由模块提供（app/bridge.js 在本脚本之前装好）。 */
/* 迁移期别名：拆分时把 document / window 统一成 doc / g（阶段 7 随文件删）。 */
const doc = globalThis.document;
const g = globalThis;
const core = globalThis.solomni;
/* 迁移期：已迁到 app/features 的入口由 app/main.js 挂到 core.app，这里按名取用（阶段 7 删）。 */
const app = core.app || (core.app = {});
const state = core.state;
const stores = core.stores;
const api = core.api;
/* 组件库（app/ui/）：按名字绑定成本文件里的局部名。 */
const { btn, busyButton, textInput, numberInput, areaInput, selectInput, field, syncSelect, checkbox, emptyHint } = ui;

/* 登记处一变，所有已打开的分页都跟着同步列表与下拉选项（app/stores/state.js 的 createRegistryWatch）。 */
const registry = stores.createRegistryWatch(state, (e) => console.error("登记处视图刷新失败：", e));
function onRegistryChange(fn) { return registry.on(fn); }
function notifyRegistryChange() { registry.notify(); }
/** 模型下拉的选项：needEmpty = 带「（核心默认）」空项（会话配置用）。 */
function modelOptions(current, needEmpty) {
  const opts = (needEmpty ? [{ value: "", label: "（核心默认）" }] : [])
    .concat(state.models.map((m) => ({ value: m.id, label: m.name + "（" + m.id + "）" })));
  if (current && !state.models.some((m) => m.id === current)) {
    opts.push({ value: current, label: current + "（登记处已无此模型）" });
  }
  return opts;
}

/* ---------- 状态与侧栏 ---------- */
/** 拉一次 /api/state：快照投影进状态（app/stores/state.js），再补新会话的盘上转录并重画。 */
async function refreshState() {
  const s = await api('GET', '/api/state');
  const opened = stores.projectSnapshot(state, s);
  // 盘上转录是新会话的前缀：补上它，之后的实时事件按 seq 接上。
  for (const cur of opened) hydrateHistory(cur);
  // 登记处真变了才通知：轮询每 3 秒都会进来，每次重画会让用户填到一半的表单被冲掉。
  const registryChanged = registry.check();
  renderFenceBanner();
  renderToolchainBanner();
  renderSidebar();
  renderHistory();
  if (opened.length) renderTabs();
  const cur = activeSession();
  if (cur) {
    syncTyping(cur); syncSendButton(cur); syncComposer(cur);
    // 提交信息不塞进每 3 秒的 /api/state：按需、低频刷新（后台 agent 提交也能跟上）。
    if (!cur.workAt || Date.now() - cur.workAt > 15000) loadWork(cur.sid);
  }
  if (registryChanged) registry.notify();
}

/// 思维链块：永远默认折叠，点击（原生 details）才展开。
/// Markdown 容器：有渲染器就渲染，没有就退回纯文本（安全）。
function mdNode(text) {
  const d = doc.createElement('div');
  d.className = 'md';
  if (typeof markdownToHtml === 'function') d.innerHTML = shortPath(markdownToHtml(text));
  else d.textContent = text;
  return d;
}

/// 正文：AI/用户发言按 Markdown 渲染，系统提示保持纯文本。
function appendBody(el, cls, text) {
  // 正文必须**追加**，不能 el.innerHTML = …：那会把上面刚挂的身份标题（.who）一起抹掉，
  // 于是 agree（绿框）这类非 line 类的消息就"没了说话人"——用户不知道这句来自谁。
  if (cls === 'line' || cls === 'plan' || cls === 'user') {
    el.appendChild(mdNode(text));
    return;
  }
  const body = doc.createElement('div');
  body.className = 'body';
  body.innerHTML = shortPath(escHtml(text)); // 系统/验收等行：纯文本转义后同样缩写长路径
  el.appendChild(body);
}

/// 纯文本转义：工具卡片的原始 JSON 一律按字面显示（<pre> 里不解释 HTML）。
function escHtml(s) {
  return String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (
    { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]
  ));
}

/* 折叠状态：会话级 store（key → bool）。renderStream 每次全量重建 DOM，
 * 所以折叠状态必须由「稳定键 + 会话级 store」自己保存与恢复——键只依赖"只追加"的顺序，
 * 刷新多少次都不变。CoT 与工具卡片的三处折叠共用同一个 store。 */
function foldOpen(sess, key) {
  if (!sess.fold) sess.fold = {};
  return !!sess.fold[key];
}

/// 绑定折叠：按 store 设初值，并在 toggle 时回写。
function foldBind(sess, key, d) {
  d.open = foldOpen(sess, key); // 先设初值再挂监听，避免程序化赋值触发一次回写
  d.addEventListener('toggle', () => {
    sess.fold = sess.fold || {};
    sess.fold[key] = d.open;
  });
  return d;
}

/* <pre> 的滚动位置也要自己存/恢复：定稿整帧重建时，不能把用户拖到一半的滚动条弹回去。
 * 键沿用折叠键 + ':scroll'；恢复必须在节点插入文档之后做（离屏元素的 scrollTop 在浏览器里不生效），
 * 所以这里只登记，由 flushScroll 在渲染收尾时统一写回。 */
let pendingScroll = [];
function scrollKey(key) { return key + ':scroll'; }
function flushScroll(sess) {
  for (const it of pendingScroll) {
    const v = sess.scroll && sess.scroll[scrollKey(it.key)];
    if (typeof v === 'number' && v > 0) it.pre.scrollTop = v;
  }
  pendingScroll = [];
}

/// 折叠的原始内容块：默认收起，点开才看（参数 / 结果 / 原文）。
/// literal = true 时逐字保留（「原文」是要给用户照着复制的记录，不做长路径缩写）。
function toolDetail(label, text, sess, key, literal) {
  const d = doc.createElement('details'); d.className = 'tool-raw';
  foldBind(sess, key, d);
  const s = doc.createElement('summary'); s.textContent = label;
  const pre = doc.createElement('pre'); pre.className = 'tool-pre';
  // 先转义再缩写：文字内容不变，只是把长路径显示成短式胶囊（「原文」保持逐字）。
  const shown = escHtml(text == null || text === '' ? '（空）' : text);
  pre.innerHTML = literal ? shown : shortPath(shown);
  const sk = scrollKey(key);
  pre.addEventListener('scroll', () => { sess.scroll = sess.scroll || {}; sess.scroll[sk] = pre.scrollTop; });
  pendingScroll.push({ pre: pre, key: key });
  d.appendChild(s); d.appendChild(pre);
  return d;
}

/// 工具调用卡片：头部只写「模块.工具名」+ 成败；参数与结果折叠在里面（折叠状态走 fold store）。
/// 流式 tool_call 与权威 transcript 的 tool 行共用这一个渲染函数，也共用同一个折叠键。
function toolCard(t, sess, key, who) {
  const info = t || {};
  const el = doc.createElement('div');
  el.className = 'line tool-card ' + (info.ok ? 'ok' : 'bad');
  el.title = info.speaker ? info.speaker + ' 调用工具' : '工具调用';
  // 左上角先标出**是谁**在调工具：与其它行同一条规则（每个 kind 都带身份）。
  const speaker = who || info.speaker || '';
  if (speaker) {
    const w = doc.createElement('span'); w.className = 'who'; w.textContent = speaker; el.appendChild(w);
  }
  const head = doc.createElement('div'); head.className = 'tool-head';
  const name = doc.createElement('span'); name.className = 'tool-name';
  name.textContent = (info.module ? info.module + '.' : '') + (info.name || '工具');
  const mark = doc.createElement('span'); mark.className = 'tool-mark'; mark.textContent = info.ok ? '✓' : '✗';
  head.appendChild(name); head.appendChild(mark);
  el.appendChild(head);
  el.appendChild(toolDetail('参数', info.args, sess, key + ':args'));
  el.appendChild(toolDetail('结果', info.output, sess, key + ':output'));
  return el;
}

/* ---------- 长路径缩写 ---------- */
/* 转录里是真实绝对路径（如 D:/…/session/总结/work/README.md），显示时缩成短式胶囊：
 * 落在共享区根之下 → work/相对路径；落在某 agent 沙箱根之下 → 沙箱 <agent>/相对路径；
 * 根还没拿到、或不在任何根之内 → **原样显示**（不做任何猜测性缩短）。完整路径放在 title 里。
 *
 * 安全前提（为什么可以对这个 HTML 字符串做替换）：md.js 只把 http/https 渲染成 href 且已转义，
 * 所以文件路径不可能出现在 HTML 属性里；字符类里排除 & 是为了不被 &quot; 这类实体的分号吞掉。 */
let shortPathRoots = null; // 当前活动会话的根列表（先长后短，agent 根排在共享区根前）


/// 从 /files 的 roots 构造匹配表；没有 roots（后端未提供 / 还没拉到）→ null = 原样显示。
function buildRootList(roots) {
  if (!roots) return null;
  const out = [];
  const add = (root, kind, name) => {
    const r = String(root == null ? '' : root).replace(/\/+$/, '');
    if (r) out.push({ root: r, kind: kind, name: name || '' });
  };
  for (const a of (roots.agents || [])) add(a && a.root, 'agent', a && a.name);
  add(roots.work, 'work');
  // 先长后短：避免父根遮住子根（agent 根先入列，同长时优先）
  out.sort((x, y) => y.root.length - x.root.length);
  return out.length ? out : null;
}

/// 把正文 HTML 里的长路径替换成短式胶囊（完整路径进 title）。
function shortPath(html) {
  const src = String(html == null ? '' : html);
  const list = shortPathRoots;
  if (!list || !list.length) return src;
  const alt = list.map((it) => app.escapeRe(it.root)).join('|');
  const re = new RegExp('(?:' + alt + ')/([^\\s<>\\x22\\x27&]*)', 'g');
  return src.replace(re, (m, rel, offset, whole) => {
    // 前面还是路径字符 → 只是一个更长路径的中间段，不动它
    const before = offset > 0 ? whole[offset - 1] : '';
    if (before && /[A-Za-z0-9_\-./\\]/.test(before)) return m;
    for (const it of list) {
      if (m.indexOf(it.root + '/') === 0) {
        const label = it.kind === 'agent' ? '沙箱 ' + it.name + '/' + rel : 'work/' + rel;
        return '<span class="ref-pill" title="' + escHtml(m) + '">' + escHtml(label) + '</span>';
      }
    }
    return m;
  });
}

/// 思维链块：永远默认折叠；展开状态记在会话的 fold store 里，流式重建也不会被收回。
function reasoningBlock(text, sess, key) {
  const d = doc.createElement('details');
  d.className = 'cot';
  foldBind(sess, key, d);
  const s = doc.createElement('summary'); s.textContent = '思维链';
  const b = doc.createElement('div'); b.className = 'cot-body'; b.textContent = text;
  d.appendChild(s); d.appendChild(b);
  return d;
}

/* ---------- 会话历史：树 + 铅笔菜单 ----------
 * 根会话可展开子会话；活跃路径自动展开。点条目直接打开会话；改配置 / 删除走行尾铅笔的浮层菜单。
 * 子会话不单独删（删父会话会一起删）。 */
function historyByParent() {
  const by = new Map();
  for (const h of (state.history || [])) {
    const p = h.parent || '';
    if (!by.has(p)) by.set(p, []);
    by.get(p).push(h);
  }
  return by;
}

/// 当前会话的祖先链（自己 → 父 → …）：树据此自动展开，用户不用找。
function activePath() {
  const out = [];
  const seen = new Set();
  let cur = state.activeSid;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    out.push(cur);
    const h = (state.history || []).find((x) => x.name === cur);
    cur = h && h.parent ? h.parent : null;
  }
  return out;
}

/// 行尾铅笔菜单：打开 / 编辑配置 / 删除（子会话多一项「打开父会话」，不给删除）。
function historyMenu(h) {
  const busy = generating(h.name);
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
      row.onclick = () => openHistory(h.name);
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
      // 运行态是**持久事实**（停止 / 关闭），与"这一刻在不在跑"分开：侧栏据此标出来。
      const stateTag = h.run === 'stopped' ? '·已停止'
        : h.run === 'closed' ? '·已关闭'
          : (h.done ? '' : '·进行中 ');
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

/// 快照（/api/state）说这条会话、或它的任一子会话在跑吗。
/// **对账副本的唯一读法**：只在 isBusy 的第三档（本页没有实时知识）用。
function snapshotRunning(sid) { return stores.snapshotRunning(state, sid); }

/// 这条会话此刻在不在干活。**运行态的唯一真相是事件**（每个生产者在起止各推一条 `Working`；
/// 刷新页面或事件台有缺口时用快照对账）——三个来源各司其职，不是三份互相打架的真话：
/// 1. `s.sending`：本页发起的命令还在途（本地事实，与服务端运行态无关）；
/// 2. `s.running` + `s.running_known`：收到过它的运行态事件 → 以事件为准，快照不得覆盖；
/// 3. 没有实时知识：用快照对账——成员回合跑在它自己的会话里，主会话整回合收不到事件，
///    只靠"有增量"猜就永远切不出「停止」按钮、也没有占位动画。
function isBusy(s) {
  if (!s) return false;
  if (s.sending) return true;
  // ① 这条会话**自己**的运行态：有实时知识（收过它的 Working）就以事件为准，否则用快照对账。
  if (s.running_known ? s.running : snapshotRunning(s.sid)) return true;
  // ② 它的**子会话**在跑也算——成员回合、节点执行都跑在各自的子会话里，主会话整回合收不到事件，
  //    所以子会话的运行态只有快照说得清；子会话标签页开着时，它自己的实时知识比快照新，优先用它。
  const pre = s.sid + '--';
  for (const k of state.running || []) {
    if (k.indexOf(pre) !== 0) continue;
    const child = state.sessions.get(k);
    if (child && child.running_known) {
      if (child.running) return true;
      continue;
    }
    return true;
  }
  return false;
}

function generating(sid) {
  const s = state.sessions.get(sid);
  return isBusy(s);
}

/* 记的是虚拟机档、但本机现在承载不了：**不拦打开**（记录是用户的），只主动把原因与出路说清。
 * 为什么必须提示：虚拟机档的 guest 本体尚未接入，此刻选它只会得到一个更差的本机档——
 * 用户有权知道这一点，也有权知道怎么才能用上。 */
function warnIfTierUnavailable(name) {
  const view = (state.history || []).find((h) => h.name === name);
  if (!view || view.tier !== 'vm' || view.tier_ready !== false) return;
  const missing = (view.tier_missing || []).join('；') || '本机不具备虚拟机档的前置条件';
  notice(
    '这条会话的虚拟机档当前不可用',
    '原因：' + missing +
      '。\n\n本次仍会打开（历史记录不受影响），但它实际按本机档执行。' +
      '\n要真正用上虚拟机档：先在本机启用虚拟化（Windows「虚拟机平台」/ Linux 的 /dev/kvm / macOS 11+），' +
      '并在「编辑」里确认执行档位与虚拟机；暂时不需要就在「编辑」里改回本机档。',
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
  } catch (err) { notice('操作失败', err.message, 'err'); }
}

async function deleteHistory(name) {
  // 外部工作区不归本产品所有：删除只删会话目录，必须先说清“工作区文件保留”。
  const sess = state.sessions.get(name);
  if (sess && sess.workspace) {
    if (!(await confirmBox('删除会话', '删除会话「' + name + '」？记录与版本库将被永久删除；工作区 ' + sess.workspace + ' 里的文件会保留，不会被删除。', '删除'))) return;
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
    } catch (err) { notice('操作失败', err.message, 'err'); }
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
    if (!(await confirmBox('删除会话与工作区文件', text, '一起删除'))) return;
  } else if (!(await confirmBox('删除会话', '删除会话「' + name + '」？该会话的记录将被永久删除。', '删除'))) {
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
  } catch (err) { notice('操作失败', err.message, 'err'); }
}

/* ---------- 配置视图（会话列表的「编辑」）：名单 / 模块 / 模型 / 档位 / 运行能力 / 定版 ----------
 * 读：GET /api/sessions/{sid}/config；写：POST /api/sessions/{sid}/edit。
 * 「重新扫描」= 重新 GET 一次并重绘：模块清单与依赖文件夹在每次读取时重扫，不需要额外接口。
 * 冻结口径与后端一致：started=true 时只冻结 agent 名单与形态（名字只读、不能增删 agent），模块与模型仍可改。
 * 保存失败一律把后端 error 原文显示出来，不在前端兜底改写。 */

/* 配置面板里的小构件：都只用冒烟桩支持的 DOM API，与向导/登记处同一写法。 */
function cfgDiv(cls, text) {
  const d = doc.createElement('div'); d.className = cls;
  if (text != null) d.textContent = text;
  return d;
}
function cfgHint(text, kind) {
  return cfgDiv('cfg-hint' + (kind ? ' ' + kind : ''), text);
}
/// 执行档位二选一：disabled = 本机承载不了（例如虚拟机档的前置条件不具备）。
const cfgRadio = ui.radio;

/// 从会话列表的「编辑」进入：面板自己拉配置、自己重绘（保存 / 重新扫描都走这里）。
function openConfig(sid) {
  openModal('配置：' + sid, (c) => {
    const box = doc.createElement('div'); box.className = 'cfg';
    c.body.appendChild(box);
    // 登记处变了只同步模型下拉；重读配置会换掉 DOM，所以这里只订阅一次，由 syncModels 指向最新一份。
    let syncModels = () => {};
    c.onRegistryChange(() => syncModels());
    loadConfig(sid, c, box, (fn) => { syncModels = fn; });
  }, true);
}

async function loadConfig(sid, c, box, setSync) {
  box.innerHTML = '';
  box.appendChild(emptyHint('正在读取配置…'));
  let cfg = null;
  try {
    const r = await api('GET', '/api/sessions/' + encodeURIComponent(sid) + '/config');
    cfg = r.config || null;
  } catch (e) {
    box.innerHTML = '';
    box.appendChild(cfgHint('读不到这个会话的配置：' + e.message, 'err'));
    return;
  }
  box.innerHTML = '';
  if (!cfg) { box.appendChild(cfgHint('后端没有返回 config', 'err')); return; }
  buildConfigForm(sid, cfg, c, box, setSync);
}

function buildConfigForm(sid, cfg, c, box, setSync) {
  const frozen = !!cfg.started;
  // 模型下拉的同步器：openConfig 的登记处订阅只调最新这一份。
  const modelSyncs = [];
  if (setSync) setSync(() => { for (const fn of modelSyncs) fn(); });
  const rt = cfg.runtime || {};
  const dir = cfg.runtimes_dir || '';
  // 表单草稿：保存从这里读；「重新扫描」整块重建，未保存的改动随之丢弃。
  const draft = {
    agents: (cfg.agents || []).map((a) => ({
      name: a.name || '', modules: (a.modules || []).slice(), model: a.model || '', chara: a.chara || '',
    })),
    tier: cfg.tier === 'vm' ? 'vm' : 'host',
    guest: cfg.guest || '',
    pins: {},
    permission: cfg.permission || { profile: 'workspace', shared: { read: 'all', write: 'all', deny: [] }, tools: {} },
  };
  const pins0 = cfg.pins || {};
  for (const k of Object.keys(pins0)) draft.pins[k] = pins0[k];

  // 身份与冻结事实
  const head = cfgDiv('cfg-sec');
  head.appendChild(cfgDiv('cfg-sec-title', '会话 ' + (cfg.sid || sid)));
  head.appendChild(cfgDiv('cfg-line', '形态：' + (cfg.mode === 'collab'
    ? '协作（N 个 agent，各自独立沙箱）'
    : '单 agent（恰好 1 个 agent，模块数不限）')));
  head.appendChild(frozen
    ? cfgHint('这轮会话已经开过：agent 名单与形态冻结 —— 名字只读、不能增删 agent（要换人请新建会话）。模块与模型仍可改。', 'warn')
    : cfgHint('这轮会话还没有内容：agent 名单与形态都还改得动。'));
  box.appendChild(head);

  // agent 名单：名字 / 模块 / 模型（同一模块可被多个 agent 共用）
  const secA = cfgDiv('cfg-sec');
  secA.appendChild(cfgDiv('cfg-sec-title', 'agent 名单（' + draft.agents.length + ' 个）'));
  if (!draft.agents.length) secA.appendChild(emptyHint('（这个会话没有 agent 记录）'));
  draft.agents.forEach((a, i) => secA.appendChild(cfgAgentBlock(a, i, frozen, modelSyncs)));
  box.appendChild(secA);

  // 执行档位：本机 / 虚拟机（如实说明各自是什么）
  const secT = cfgDiv('cfg-sec');
  secT.appendChild(cfgDiv('cfg-sec-title', '执行档位'));
  // 虚拟机档的**承载**由后端判定（与「开始」/保存同一把尺子）：前置条件不具备时禁用，不让选。
  // 与「未接入」是两回事：guest 本体尚未接入这一点写在下面的提示里，两条都如实说。
  const vmOk = cfg.vm_available !== false;
  const tierLocked = !!cfg.tier_locked;
  const hostR = cfgRadio('本机档 —— 脚本直接在宿主上跑：按宿主目标装载 runtimes/ 的包，基础工具链 / 宿主 PATH 回退；隔离就是宿主本身（快，但风险也在宿主上）。', draft.tier === 'host', 'cfg-tier');
  const vmR = cfgRadio('虚拟机档 —— 一整套 guest，脚本在 guest 里跑：按模块声明的运行能力装载运行包；隔离更强，代价是更重、依赖运行包。', draft.tier === 'vm', 'cfg-tier', !vmOk);
  if (tierLocked) {
    hostR.box.disabled = true;
    vmR.box.disabled = true;
    secT.appendChild(cfgHint('执行档位随主会话：子会话不能单独改档位与虚拟根。', 'warn'));
  }
  // 虚拟机档选的是**虚拟根**（清单里的一个虚拟机），不是路径：本机有哪些由后端给。
  let guestInfo = (cfg.guests || []).find((g) => g.guest === draft.guest) || null;
  let syncGuestSetup = () => {};
  const pick = guestPicker(cfg.guests, draft.guest, (v, g) => {
    draft.guest = v; guestInfo = g; syncGuestSetup();
  });
  // 机器前置齐、但选中的虚拟根没装齐时，这里给同一个补齐动作（与上面的机器前置入口不重复）。
  const guestSetup = app.setupAction(null, () => openConfig(sid));
  guestSetup.classList.add('hidden');
  syncGuestSetup = () => {
    const show = vmOk && draft.tier === 'vm' && (!guestInfo || !guestInfo.ready);
    if (show) guestSetup.classList.remove('hidden');
    else guestSetup.classList.add('hidden');
  };
  const syncTier = () => {
    const vm = draft.tier === 'vm';
    const show = vm && !tierLocked;
    pick.wrap.className = show ? 'wf-field' : 'wf-field hidden';
    pick.hint.className = 'cfg-hint' + (show ? '' : ' hidden');
    pick.sync();
    syncGuestSetup();
  };
  hostR.box.addEventListener('change', () => { if (hostR.box.checked) { draft.tier = 'host'; syncTier(); } });
  vmR.box.addEventListener('change', () => { if (vmR.box.checked) { draft.tier = 'vm'; syncTier(); } });
  secT.appendChild(hostR.wrap); secT.appendChild(vmR.wrap);
  // 前置逐项照抄后端（缺哪几项、每项怎么补），界面不自己编话、也不笼统说"前置条件不具备"。
  const reqs = cfg.vm_requirements || [];
  if (!vmOk) {
    secT.appendChild(cfgHint('虚拟机档现在不能选：' + (cfg.vm_unavailable_reason || '前置条件不具备'), 'err'));
    if (reqs.length) {
      const box2 = cfgDiv('cfg-sec');
      box2.appendChild(cfgDiv('cfg-sec-title', '虚拟机档前置（' + reqs.filter((r) => !r.met).length + ' 项未满足）'));
      for (const r of reqs) {
        const line = doc.createElement('div');
        line.className = 'cfg-hint' + (r.met ? '' : ' err');
        line.textContent =
          (r.met ? '✓ ' : '✗ ') + r.detail + (r.met ? '' : '　怎么补：' + r.how);
        box2.appendChild(line);
      }
      secT.appendChild(box2);
    }
    // 缺什么已经列出来了，这里给**一个动作**（不是一句命令行）。
    // 装完重开配置面板：补齐弹层会把 #modal-root 顶掉，旧面板的 DOM 已经不在了——
    // 与其往脱离文档的节点上重绘，不如按新状态整份重读（未保存的改动会丢，和「重新扫描」同一个代价）。
    secT.appendChild(app.setupAction(null, () => openConfig(sid)));
  }
  secT.appendChild(pick.wrap); secT.appendChild(pick.hint); secT.appendChild(guestSetup);
  box.appendChild(secT);
  syncTier();

  // 运行能力：declared / missing / available / diagnoses / rejected / rejected_packages
  box.appendChild(cfgRuntimeBlock(rt, dir, draft.tier, () => {
    c.setMsg('正在重新扫描模块清单与依赖文件夹…');
    loadConfig(sid, c, box, setSync);
  }));

  // 定版：只对「有多个可用版本」的能力给选择
  const secP = cfgDiv('cfg-sec');
  secP.appendChild(cfgDiv('cfg-sec-title', '定版（同一能力有多个可用版本时才需要选）'));
  const avail = rt.available || {};
  const multiCaps = Object.keys(avail).filter((cap) => (avail[cap] || []).length > 1).sort();
  const singleCaps = Object.keys(avail).filter((cap) => (avail[cap] || []).length === 1).sort();
  if (!multiCaps.length) {
    secP.appendChild(cfgHint(singleCaps.length
      ? '（' + singleCaps.join('、') + ' 只有一个可用版本，不需要定版）'
      : '（还没有可用的多版本能力：依赖文件夹里的能力都只有一个版本，或包库里还没有运行包）'));
  }
  for (const cap of multiCaps) {
    const vs = (avail[cap] || []).slice();
    const cur = draft.pins[cap] || '';
    const opts = [{ value: '', label: '（不定版：用包库里第一个 ' + (vs[0] || '') + '）' }]
      .concat(vs.map((v) => ({ value: v, label: v })));
    if (cur && vs.indexOf(cur) < 0) opts.push({ value: cur, label: cur + '（包库里现在没有这个版本）' });
    const sel = selectInput(opts, cur);
    sel.addEventListener('change', () => { draft.pins[cap] = sel.value; });
    secP.appendChild(field(cap + ' 的版本', sel));
  }
  // 单版本 / 看不见的能力：不给选择，但要如实说清现状（不静默丢掉已有的定版）
  for (const cap of singleCaps) {
    if (draft.pins[cap]) {
      secP.appendChild(cfgHint('已定版：' + cap + ' = ' + draft.pins[cap] + '（这个能力只有一个可用版本，不需要选）'));
    }
  }
  for (const cap of Object.keys(draft.pins)) {
    if (!avail[cap]) {
      secP.appendChild(cfgHint('已定版：' + cap + ' = ' + draft.pins[cap] + '（包库里现在看不到这个能力，确认版本或重新扫描）', 'warn'));
    }
  }
  box.appendChild(secP);

  // 会话权限：四档 + 共享区三态 + 逐工具决定（网络是全局开关，不在这里）
  const secPerm = cfgDiv('cfg-sec');
  secPerm.appendChild(cfgDiv('cfg-sec-title', '会话权限'));
  secPerm.appendChild(cfgHint('网络由全局 sandbox.policy.network 控制；本会话不可单独改。子会话只能换更低档、只能收窄。'));
  const perm = draft.permission || {};
  const parseLines = (text) => String(text || '')
    .split(/[\n,]/).map((s) => s.trim()).filter((s) => s.length > 0);
  const profiles = (cfg.permission && cfg.permission.profiles) || [
    { id: 'view', label: '仅可查看', summary: '' },
    { id: 'workspace', label: '工作区内修改', summary: '' },
    { id: 'external', label: '外部可达（逐次确认）', summary: '' },
    { id: 'all_reach', label: '全可达（自动放行）', summary: '' },
  ];
  const curProfile = perm.profile || 'workspace';
  const parentProfile = (cfg.permission && cfg.permission.parent_profile) || null;
  const parentIndex = parentProfile ? profiles.findIndex((p) => p.id === parentProfile) : profiles.length - 1;
  const prof = selectInput(profiles.map((p) => ({ value: p.id, label: p.label })), curProfile);
  Array.from(prof.children || []).forEach((o, i) => {
    if (parentProfile && i > parentIndex) o.disabled = true;
  });
  secPerm.appendChild(field('权限档位', prof));
  const profHint = cfgHint('');
  secPerm.appendChild(profHint);
  const syncProf = () => {
    const p = profiles.find((x) => x.id === prof.value);
    profHint.textContent = p ? p.summary : '';
  };
  prof.addEventListener('change', syncProf);
  syncProf();
  const makeList = (label, value, hint) => {
    const t = areaInput('每行一条');
    t.value = Array.isArray(value) ? value.join('\n') : '';
    t.addEventListener('input', () => { if (hint) hint.textContent = ''; });
    secPerm.appendChild(field(label, t));
    return t;
  };
  const scopeEditor = (label, value) => {
    const mode = Array.isArray(value) ? 'paths' : (value === 'none' ? 'none' : 'all');
    const sel = selectInput([
      { value: 'all', label: '整棵共享区' },
      { value: 'none', label: '无权限' },
      { value: 'paths', label: '指定路径' },
    ], mode);
    const area = areaInput('每行一条（相对共享区根）');
    area.value = Array.isArray(value) ? value.join('\n') : '';
    const wrap = cfgDiv('cfg-sec');
    wrap.appendChild(field(label, sel));
    const areaField = field(label + '：路径', area);
    wrap.appendChild(areaField);
    const sync = () => { areaField.style.display = sel.value === 'paths' ? '' : 'none'; };
    sel.addEventListener('change', sync);
    sync();
    secPerm.appendChild(wrap);
    return () => (sel.value === 'all' ? 'all' : sel.value === 'none' ? 'none' : parseLines(area.value));
  };
  const getRead = scopeEditor('共享区读范围', (perm.shared && perm.shared.read) || 'all');
  const getWrite = scopeEditor('共享区写范围', (perm.shared && perm.shared.write) || 'all');
  const deny = makeList('追加黑名单（读写都禁；全局黑名单是硬底线，这里只能追加）', (perm.shared && perm.shared.deny) || []);
  const toolsArea = areaInput('每行一条：工具名=allow/ask/deny');
  toolsArea.value = Object.entries(perm.tools || {}).map(([k, v]) => k + '=' + v).join('\n');
  secPerm.appendChild(field('逐工具决定（可选；子会话不能改）', toolsArea));
  const parseTools = () => {
    const out = {};
    for (const line of parseLines(toolsArea.value)) {
      const i = line.indexOf('=');
      if (i < 0) continue;
      const k = line.slice(0, i).trim();
      const v = line.slice(i + 1).trim();
      if (k && ['allow', 'ask', 'deny'].includes(v)) out[k] = v;
    }
    return out;
  };
  box.appendChild(secPerm);

  // 思考强度：工作根一个档（子会话共享）；改了就生效，与状态栏 chip 走同一个动作。
  const secThink = cfgDiv('cfg-sec');
  secThink.appendChild(cfgDiv('cfg-sec-title', '思考强度（工作根共享）'));
  const think = cfg.thinking || { level: null, default_level: null, label: '未设置', options: [], note: '' };
  const thinkOpts = (think.options || []).map((o) => ({ value: o.id, label: o.label }));
  const current = think.level || think.default_level || '';
  // 只有没有统一默认档时，才需要「跟随默认」这一项；有具体默认档时它本身就是一项。
  if (!think.default_level) thinkOpts.unshift({ value: '', label: '（跟随默认）' });
  const known = (think.options || []).some((o) => o.id === current);
  if (current && !known) {
    thinkOpts.unshift({ value: current, label: (think.label || current) + '（当前档不可用）' });
  }
  const thinkSel = selectInput(thinkOpts, current);
  if (current && !known && thinkSel.children[0]) thinkSel.children[0].disabled = true;
  thinkSel.onchange = async () => {
    const picked = thinkSel.value;
    const level = picked === think.default_level ? '' : picked;
    try {
      await api('POST', actionUrl('set_thinking'), { session_id: sid, level });
      await refreshState();
      c.setMsg(level ? '思考强度：' + thinkingLabel(level) : '思考强度：跟随模型默认');
      loadConfig(sid, c, box, setSync);
    } catch (e) { c.setMsg(e.message, true); }
  };
  secThink.appendChild(field('思考强度', thinkSel));
  secThink.appendChild(cfgHint(think.note || '只作用于对话生成；核心内部编排保持供应商默认。只列这组模型都登记了的档；选中共同默认档 = 跟随模型默认。'));
  box.appendChild(secThink);

  // 动作
  const acts = cfgDiv('cfg-actions');
  const save = btn('保存', 'btn btn-primary');
  const cancel = btn('取消', 'btn btn-ghost');
  cancel.onclick = closeModal;
  save.onclick = async () => {
    const pins = {}; // 空串 = 不定版：不提交（提交空串会被当成"定版到空版本"）
    for (const cap of Object.keys(draft.pins)) {
      const v = draft.pins[cap];
      if (v) pins[cap] = v;
    }
    const body = {
      agents: draft.agents.map((a) => ({ name: a.name.trim(), modules: a.modules.slice(), model: a.model || '', chara: a.chara || '' })),
      tier: draft.tier,
      guest: draft.tier === 'vm' ? (draft.guest || null) : null,
      pins: pins,
      permission: {
        profile: prof.value,
        shared: {
          read: getRead(),
          write: getWrite(),
          deny: parseLines(deny.value),
        },
        tools: (cfg.permission && cfg.permission.narrowed) ? {} : parseTools(),
      },
    };
    save.disabled = true;
    c.setMsg('保存中…');
    try {
      await api('POST', actionUrl('edit_session'), Object.assign({ session_id: sid }, body));
      await refreshState();
      c.setMsg('已保存：下一次发言按新配置生效');
      loadConfig(sid, c, box, setSync); // 重读一遍：把落盘后的样子如实显示出来
    } catch (e) {
      save.disabled = false;
      c.setMsg('保存失败：' + e.message, true); // 后端 error 原文，不吞
    }
  };
  acts.appendChild(save); acts.appendChild(cancel);
  box.appendChild(acts);
}

/// 一个 agent 的编辑块：名字（冻结时只读）/ 模块 chips（选项来自 /api/state 的 modules）/ 人设 / 模型下拉。
function cfgAgentBlock(a, idx, frozen, modelSyncs) {
  const row = cfgDiv('cfg-agent');
  row.appendChild(cfgDiv('cfg-agent-head', 'agent ' + (idx + 1) + '：' + (a.name || '（未命名）')));

  const nameIn = textInput('agent 名字（也会当它的沙箱目录名）');
  nameIn.value = a.name || '';
  nameIn.disabled = frozen;
  nameIn.addEventListener('input', () => { a.name = nameIn.value; });
  row.appendChild(field(frozen ? '名字（只读）' : '名字', nameIn));
  if (frozen) row.appendChild(cfgHint('会话已经开过：名单与形态冻结 —— 名字改不了（要换人请新建会话）；模块与模型仍可改。', 'warn'));

  const chips = app.moduleChips({
    selected: () => a.modules,
    onChange: (ids) => { a.modules = ids; },
  });
  row.appendChild(field('模块（这个 agent 的能力；同一模块可被多个 agent 共用）', chips.node));
  const charaIn = areaInput('人设 / 身份（只影响这个会话里的这份 copy，不写回登记处）');
  charaIn.value = a.chara || '';
  charaIn.addEventListener('input', () => { a.chara = charaIn.value; });
  row.appendChild(field('人设 / 身份', charaIn));
  const cur = a.model || '';
  const sel = selectInput(modelOptions(cur, true), cur);
  sel.addEventListener('change', () => { a.model = sel.value; });
  // 登记处变了就换选项，保留这一席当前选择（值没了就如实显示"登记处已无此模型"）。
  if (modelSyncs) modelSyncs.push(() => syncSelect(sel, modelOptions(a.model || '', true), a.model || ''));
  row.appendChild(field('模型（留空 = 用核心默认）', sel));
  return row;
}

/// 运行能力区：声明 / 缺口 / 可用 / 虚拟机档诊断 / 两类拒收，全部如实呈现。
function cfgRuntimeBlock(rt, dir, savedTier, rescan) {
  const sec = cfgDiv('cfg-sec');
  const head = cfgDiv('cfg-sec-head');
  head.appendChild(cfgDiv('cfg-sec-title', '运行能力（模块声明 / 包库可用 / 缺什么）'));
  const again = btn('重新扫描', 'btn');
  again.title = '重新读取这个会话的配置（模块清单与依赖文件夹每次读取都重扫，放入即出现）；未保存的改动会随之丢弃';
  again.onclick = rescan;
  head.appendChild(again);
  sec.appendChild(head);

  sec.appendChild(cfgDiv('cfg-line', '依赖文件夹（运行包放这里）：'));
  sec.appendChild(cfgDiv('cfg-dir', dir || '（后端没有给出依赖文件夹路径）'));
  sec.appendChild(cfgHint(savedTier === 'host'
    ? '已保存的是本机档：运行包按宿主目标装载，包 bin/ 进本次执行的 PATH；缺包或缺本机载荷 = 该模块的工具不执行（会话照样进行，如实说明缺什么）。'
    : '已保存的是虚拟机档：按模块声明的运行能力装载运行包；缺包 = 该模块的工具不执行（会话照样进行，如实说明缺什么）。'));

  const declared = rt.declared || {};
  const dIds = Object.keys(declared);
  sec.appendChild(cfgDiv('cfg-sub', '模块声明的运行能力'));
  if (!dIds.length) sec.appendChild(cfgDiv('cfg-line', '（没有模块声明运行能力：都不需要运行包）'));
  for (const id of dIds) sec.appendChild(cfgDiv('cfg-line', '· 模块 ' + id + ' 需要：' + (declared[id] || []).join('、')));

  const missing = rt.missing || {};
  const mIds = Object.keys(missing);
  sec.appendChild(cfgDiv('cfg-sub', '缺口（本档位用不了的能力）'));
  if (!mIds.length) {
    sec.appendChild(cfgDiv('cfg-line cfg-ok', '没有缺口：模块声明的运行能力在依赖文件夹里都能找到。'));
  } else {
    for (const id of mIds) sec.appendChild(cfgDiv('cfg-line cfg-bad', '· 模块 ' + id + ' 缺：' + (missing[id] || []).join('、')));
    sec.appendChild(cfgHint('把适用于本档位的运行包放进 ' + (dir || '依赖文件夹 runtimes/') + '，再点上面的「重新扫描」（放入即出现；活动会话重新启用后生效）。', 'warn'));
  }

  const avail = rt.available || {};
  const aCaps = Object.keys(avail);
  sec.appendChild(cfgDiv('cfg-sub', '包库里可用'));
  if (!aCaps.length) sec.appendChild(cfgDiv('cfg-line', '（依赖文件夹里没有可用的运行包）'));
  for (const cap of aCaps) sec.appendChild(cfgDiv('cfg-line', '· ' + cap + '：' + (avail[cap] || []).join('、')));

  const diags = rt.diagnoses || [];
  if (diags.length) {
    sec.appendChild(cfgDiv('cfg-sub', '运行能力诊断（选型不成立的原因）'));
    for (const d of diags) sec.appendChild(cfgDiv('cfg-line cfg-bad', '· ' + app.diagnosisText(d)));
  }

  const rej = rt.rejected || [];
  const rejPk = rt.rejected_packages || [];
  sec.appendChild(cfgDiv('cfg-sub', '被拒收的模块 / 运行包（原因原文）'));
  if (!rej.length && !rejPk.length) sec.appendChild(cfgDiv('cfg-line', '没有被拒收的模块或运行包。'));
  for (const x of rej) sec.appendChild(cfgDiv('cfg-line cfg-bad', '· 模块：' + x));
  for (const x of rejPk) sec.appendChild(cfgDiv('cfg-line cfg-bad', '· 运行包：' + x));
  return sec;
}

/// 围栏横幅：只报“本进程降级”或“启动要求未满足”；用户主动选的 best_effort 不在这里打扰。
function renderFenceBanner() {
  const box = $('#fence-banner');
  const text = $('#fence-text');
  if (!box || !text) return;
  const f = state.fence || {};
  const missing = Array.isArray(f.sandbox_missing) ? f.sandbox_missing : [];
  const degraded = !!f.sandbox_degraded;
  if (!degraded && !missing.length) {
    box.className = 'fence-banner hidden';
    text.textContent = '';
    return;
  }
  box.className = degraded ? 'fence-banner' : 'fence-banner fence-err';
  if (degraded) {
    text.textContent = '本进程已降级：强制围栏未全部生效'
      + (missing.length ? '（' + missing.join('、') + '）' : '')
      + '；重启后重新评估。';
  } else {
    text.textContent = '启动要求未满足：' + missing.join('、') + '；模块工具会被拒绝执行。';
  }
}

/// 基础工具链横幅：项目内缺失时列出缺失能力、宿主回退是否可授权、补全命令。
function renderToolchainBanner() {
  const box = $('#toolchain-banner');
  const text = $('#toolchain-text');
  if (!box || !text) return;
  const t = state.toolchains || {};
  const missing = Array.isArray(t.missing) ? t.missing : [];
  if (!missing.length) {
    box.className = 'fence-banner hidden';
    text.textContent = '';
    return;
  }
  const fallback = t.fallback || {};
  const bad = missing.filter((id) => fallback[id] && fallback[id].fenced === false);
  const absent = missing.filter((id) => !fallback[id]);
  let msg = '基础工具链未安装：' + missing.join('、') + '。补全：node start.js --provision --ensure=toolchain --yes';
  if (bad.length) msg += ' 宿主回退在围栏下不可授权（' + bad.join('、') + '）；工具调用会问是否本次无围栏跑。';
  if (absent.length) msg += ' 宿主 PATH 也没有：' + absent.join('、') + '。';
  text.textContent = msg;
  box.className = 'fence-banner';
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
}
function closeDrawer() {
  const sb = $('#sidebar'); if (sb) sb.classList.remove('open');
  const mask = $('#drawer-mask'); if (mask) mask.classList.remove('show');
}
function toggleSidebar() {
  if (isNarrow()) { openDrawer(); return; }
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

/* ---------- 通用小构件：组件库在 app/ui/（btn / field / 各类输入见文件顶部的绑定） ---------- */
/* 虚拟根选择器：选的是**虚拟机**（清单里的一个虚拟根），不是路径——界面从不让人填目录。
 * 每一项如实标注：没装齐（缺哪几条）、各自是什么系统——界面照抄后端事实，不自己编话。 */
function guestPicker(guests, current, onChange) {
  const list = guests || [];
  const opts = list.map((g) => ({
    value: g.guest,
    label: g.label + (g.guest !== g.label ? "（" + g.guest + "）" : "") +
      (g.ready ? "" : " —— 未装齐，先补齐依赖"),
  }));
  if (current && !opts.some((o) => o.value === current)) {
    opts.push({ value: current, label: current + "（本机现在没有这个虚拟根）" });
  }
  if (!opts.length) opts.push({ value: "", label: "（本机没有可选虚拟根：先补齐依赖）" });
  const sel = selectInput(opts, current || opts[0].value);
  const wrap = field("虚拟机（虚拟根）", sel);
  const hint = cfgHint("");
  const sync = () => {
    const g = list.find((x) => x.guest === sel.value);
    hint.textContent = !g
      ? "这个虚拟根在本机找不到：点「补齐虚拟机依赖」装进项目内，或换一个。"
      : (g.os + " / " + g.arch) +
        (g.ready ? "：已装齐。" : "：还没装齐（缺 " + (g.missing || []).join("、") + "）——点「补齐虚拟机依赖」装上再用。") +
        ((g.runtimes || []).length
          ? " 自带运行能力：" + g.runtimes.join("、") + "。"
          : " 它没有声明自带的运行能力。");
    if (onChange) onChange(sel.value, g);
  };
  sel.addEventListener("change", sync);
  sync();
  return { wrap: wrap, sel: sel, hint: hint, sync: sync };
}

/* ---------- 模块选择：已选 chips + 可搜索弹层 ---------- */
/** 一行预览：把多行压成一行，过长截断（卡片 / 清单用，不冒充全文）。 */
function oneLine(text, max) {
  const s = String(text == null ? '' : text).replace(/\s+/g, ' ').trim();
  const n = max || 80;
  return s.length > n ? s.slice(0, n) + '…' : s;
}
/* ---------- 屏幕居中的提示弹窗（自研，顶替原生 alert/confirm） ----------
 * 为什么自己做：原生弹窗样式不可控、会阻塞主线程、在受限环境（内嵌浏览器 / 移动端 webview）里表现不一。
 * 与主区弹层（openModal）分开：那个在主区里、靠上对齐；这个固定在视口正中，任何布局下都在屏幕中间。
 * 三档 kind：info / warn / err（只影响配色），都只给一个「知道了」；confirmBox 返回 Promise<boolean>。
 */
function notice(title, text, kind) {
  const root = $("#notice-root");
  root.innerHTML = "";
  root.className = "notice-root open";
  const overlay = doc.createElement("div"); overlay.className = "notice-overlay";
  const panel = doc.createElement("div"); panel.className = "notice-panel notice-" + (kind || "info");
  const head = doc.createElement("div"); head.className = "notice-title"; head.textContent = title || "";
  const body = doc.createElement("div"); body.className = "notice-text"; body.textContent = text || "";
  const row = doc.createElement("div"); row.className = "notice-actions";
  const ok = btn("知道了", "btn btn-primary");
  let unbind = null;
  const close = () => {
    if (unbind) { unbind(); unbind = null; }
    root.innerHTML = "";
    root.className = "notice-root";
  };
  ok.onclick = close;
  row.appendChild(ok);
  panel.appendChild(head); panel.appendChild(body); panel.appendChild(row);
  overlay.appendChild(panel);
  root.appendChild(overlay);
  unbind = ui.bindDismiss(overlay, panel, close);
  return close;
}

/// 需要用户二选一的确认（顶替原生 confirm）：返回 Promise<boolean>，只认"确认"或"取消"。
function confirmBox(title, text, confirmLabel, danger) {
  return new Promise((resolve) => {
    const root = $("#notice-root");
    root.innerHTML = "";
    root.className = "notice-root open";
    const overlay = doc.createElement("div"); overlay.className = "notice-overlay";
    const panel = doc.createElement("div"); panel.className = "notice-panel notice-warn";
    const head = doc.createElement("div"); head.className = "notice-title"; head.textContent = title || "";
    const body = doc.createElement("div"); body.className = "notice-text" + (danger ? " notice-danger" : ""); body.textContent = text || "";
    const row = doc.createElement("div"); row.className = "notice-actions";
    let unbind = null;
    const done = (v) => {
      if (unbind) { unbind(); unbind = null; }
      root.innerHTML = "";
      root.className = "notice-root";
      resolve(v);
    };
    const no = btn("取消", "btn"); no.onclick = () => done(false);
    const yes = btn(confirmLabel || "确认", "btn btn-primary"); yes.onclick = () => done(true);
    row.appendChild(no); row.appendChild(yes);
    panel.appendChild(head); panel.appendChild(body); panel.appendChild(row);
    overlay.appendChild(panel);
    root.appendChild(overlay);
    unbind = ui.bindDismiss(overlay, panel, () => done(false));
  });
}

/* ---------- 主区弹层：可叠层（向导 → agent → 模块选择器） ----------
 * openModal 保持原语义：关掉所有层再开一层（现有调用点不需要改）。
 * openModalLayer 叠一层：Esc / 点遮罩 / ctx.close 只关最上面这一层，下层状态还在。
 */
let modalLayers = [];
function detachNode(n) {
  if (!n) return;
  if (n.parentNode && n.parentNode.removeChild) n.parentNode.removeChild(n);
  else if (typeof n.remove === 'function') n.remove();
}
/** 关掉这一层及其上面的所有层；closeModal 就是从最底层开始关。 */
function closeModalLayer(layer) {
  const at = modalLayers.indexOf(layer);
  if (at < 0) return;
  while (modalLayers.length > at) {
    const top = modalLayers.pop();
    if (top.unbind) top.unbind();
    for (const off of top.disposers.splice(0)) {
      try { off(); } catch (e) { console.warn("退订失败：", e); }
    }
    detachNode(top.overlay);
  }
  const root = $('#modal-root');
  if (!modalLayers.length) {
    root.innerHTML = '';
    root.className = 'modal-root';
  }
}
/** 关掉所有弹层（原有 closeModal 的语义）。 */
function closeModal() {
  if (!modalLayers.length) {
    const root = $('#modal-root');
    if (root) { root.innerHTML = ''; root.className = 'modal-root'; }
    return;
  }
  closeModalLayer(modalLayers[0]);
}
/** 叠一层（没有下层时就是普通弹层）。返回的 ctx.close 只关自己这一层。 */
function openModalLayer(title, build, wide) {
  const root = $('#modal-root');
  root.className = 'modal-root open';
  const overlay = doc.createElement('div'); overlay.className = 'modal-overlay';
  const panel = doc.createElement('div'); panel.className = 'modal-panel' + (wide ? ' wide' : '');
  const head = doc.createElement('div'); head.className = 'modal-head';
  const h = doc.createElement('div'); h.className = 'modal-title'; h.textContent = title;
  const close = btn('✕', 'modal-close');
  const layer = { overlay, disposers: [], unbind: null };
  const closeThis = () => closeModalLayer(layer);
  close.onclick = closeThis;
  head.appendChild(h); head.appendChild(close);
  const body = doc.createElement('div'); body.className = 'modal-body';
  const msg = doc.createElement('div'); msg.className = 'modal-msg';
  panel.appendChild(head); panel.appendChild(body); panel.appendChild(msg);
  overlay.appendChild(panel);
  root.appendChild(overlay);
  layer.unbind = ui.bindDismiss(overlay, panel, closeThis);
  modalLayers.push(layer);
  const ctx = {
    body,
    setMsg: (t, isErr) => { msg.textContent = t || ''; msg.className = isErr ? 'modal-msg err' : 'modal-msg'; },
    close: closeThis,
    // 订阅登记处变化；关层时统一退订。
    onRegistryChange: (fn) => {
      const off = onRegistryChange(fn);
      layer.disposers.push(off);
      return off;
    },
    // 登记处写操作只有这一条：动作 → 重拉状态（指纹变了会通知所有已打开的分页）。
    mutate: async (action, body) => {
      const out = await api('POST', actionUrl(action), body);
      await refreshState();
      return out;
    },
  };
  build(ctx);
  return ctx;
}
/** 开一层新的主弹层（保留原有替换语义）。 */
function openModal(title, build, wide) {
  closeModal();
  return openModalLayer(title, build, wide);
}


/* ---------- 新建工作向导 ---------- */
async function openWizard(draft) {
  // draft = 上一次没提交的草稿：补齐依赖后要把向导按原样重开，别把用户已经填的东西丢掉。
  // onclick 直接挂本函数时第一个参数是事件对象，所以按**标记**认草稿，不按真值认。
  const resumed = !!(draft && draft.__wizardDraft);
  // 两种形态共用一份 agent 卡片列表：single 恰好 0/1 个；collab 可多个。
  const w = resumed ? draft : {
    mode: 'single', agents: [], task: '', tier: 'host', guest: '', permission: 'workspace', __wizardDraft: true,
  };
  if (!Array.isArray(w.agents)) w.agents = [];
  // 名称与需求是**弹层里**输入框的现值（在本作用域之外）：由弹层装好后把取值钩子挂上来。
  let snapshotDraft = () => {};
  // 档位选择：默认档与虚拟机档可用性由后端给（GET /api/tiers，与「开始」的校验同源）。
  let tiers = null;
  try { const r = await api('GET', '/api/tiers'); tiers = (r && r.tiers) || null; } catch (e) { tiers = null; }
  if (!resumed && tiers && tiers.default) w.tier = tiers.default;
  // 档位那一格：虚拟机档不可用时禁用，并逐项说明缺什么、怎么补（照抄后端事实，不自己编话）。
  const tierField = doc.createElement('div'); tierField.className = 'wf-field';
  const tierLabel = doc.createElement('div'); tierLabel.className = 'wf-label'; tierLabel.textContent = '执行档位';
  const hostR = cfgRadio('本机档 —— 脚本直接在宿主上跑：宿主自备解释器；隔离就是宿主本身（默认）。', w.tier !== 'vm', 'wf-tier');
  const vmOk = !!(tiers && tiers.vm_available);
  const vmR = cfgRadio('虚拟机档 —— 一整套 guest，隔离更强；按模块声明装载运行包，依赖 guest 本体。', w.tier === 'vm', 'wf-tier', !vmOk);
  hostR.box.addEventListener('change', () => { if (hostR.box.checked) { w.tier = 'host'; syncWizardTier(); } });
  vmR.box.addEventListener('change', () => { if (vmR.box.checked) { w.tier = 'vm'; syncWizardTier(); } });
  tierField.appendChild(tierLabel); tierField.appendChild(hostR.wrap); tierField.appendChild(vmR.wrap);
  const profileField = doc.createElement('div'); profileField.className = 'wf-field';
  const profileLabel = doc.createElement('div'); profileLabel.className = 'wf-label'; profileLabel.textContent = '权限档位';
  const profileSel = selectInput([
    { value: 'view', label: '仅可查看' },
    { value: 'workspace', label: '工作区内修改（默认）' },
    { value: 'external', label: '外部可达（逐次确认）' },
    { value: 'all_reach', label: '全可达（自动放行）' },
  ], w.permission || 'workspace');
  profileSel.addEventListener('change', () => { w.permission = profileSel.value; });
  profileField.appendChild(profileLabel); profileField.appendChild(profileSel);
  profileField.appendChild(cfgHint('档位只决定能到哪、要不要问；创建后仍可在会话配置里收窄。代理形态默认全可达。'));
  if (!vmOk) {
    const reqs = (tiers && tiers.vm_requirements) || [];
    // 只列**现状**：怎么补这件事由下面的按钮承担（把指引和动作分成两件事，用户不会只看到话看不到路）。
    const lines = reqs.filter((r) => !r.met).map((r) => '· ' + r.detail);
    tierField.appendChild(cfgHint(
      '虚拟机档现在不能选：' + ((tiers && tiers.vm_unavailable_reason) || '本机不具备虚拟机档的前置条件') +
      (lines.length ? '\n' + lines.join('\n') : ''),
      'err'
    ));
    // 装完把向导按草稿重开：判据是后端按新状态算的，界面不自己改结论。
    // 先把用户填的名称与需求抄进草稿（它们只在弹层里读得到），否则重开会把它们清掉。
    tierField.appendChild(app.setupAction(snapshotDraft, () => openWizard(w)));
  }

  // 虚拟机档在**创建时**就要选一个虚拟根（会话级选择；机器能力已在上面判过）。
  let syncGuestSetup = () => {};
  const pick = guestPicker(tiers && tiers.guests, w.guest, (v, g) => {
    w.guest = v; w.guestInfo = g; syncGuestSetup();
  });
  // 机器前置齐、但选中的虚拟根没装齐时，虚拟根选择器下面给同一个补齐动作（与机器前置入口不重复）。
  const guestSetup = app.setupAction(snapshotDraft, () => openWizard(w));
  guestSetup.classList.add('hidden');
  syncGuestSetup = () => {
    const show = vmOk && w.tier === 'vm' && (!w.guestInfo || !w.guestInfo.ready);
    if (show) guestSetup.classList.remove('hidden');
    else guestSetup.classList.add('hidden');
  };
  // 预判：这个虚拟根（+ 包库）满足不了本次点到的哪些模块——**只是提示**，判据仍以创建时的后端校验为准。
  const gapHint = cfgHint('');
  function syncGuestGap() {
    if (w.tier !== 'vm') { gapHint.textContent = ''; return; }
    const g = w.guestInfo;
    if (!g || !g.ready) { gapHint.textContent = ''; return; }
    const covered = {};
    (g.runtimes || []).forEach((c) => { covered[c] = true; });
    Object.keys((tiers && tiers.available) || {}).forEach((c) => { covered[c] = true; });
    const ids = w.agents.reduce((acc, a) => acc.concat(a.modules || []), []);
    const lack = [];
    for (const id of ids) {
      const m = state.modules.find((x) => x.id === id);
      if (!m) continue;
      const miss = (m.runtimes || []).filter((c) => !covered[c]);
      if (miss.length) lack.push(id + ' 需要 ' + miss.join('/'));
    }
    gapHint.textContent = lack.length
      ? '这个虚拟机可能要缺：' + lack.join('；') + '。可选：把运行包放进 runtimes/、换一个虚拟机、或改用本机档（创建时后端会按同一份事实再判一次）。'
      : '';
  }
  function syncWizardTier() {
    pick.wrap.className = w.tier === 'vm' ? 'wf-field' : 'wf-field hidden';
    pick.hint.className = 'wf-hint' + (w.tier === 'vm' ? '' : ' hidden');
    gapHint.className = 'wf-hint' + (w.tier === 'vm' ? '' : ' hidden');
    pick.sync();
    syncGuestSetup();
    syncGuestGap();
  }
/* 选择工作区目录：目录列表来自本机 Web 的 /api/dirs；只选路径，不开文件。 */
function pickWorkspaceDir(onPick) {
  let parentPath = null;
  // 叠在“新建工作”之上开一层：关掉选择器后创建表单还在。
  openModalLayer('选择工作区目录', (c) => {
    const pathIn = textInput('当前目录');
    pathIn.readOnly = true;
    const rootsRow = doc.createElement('div');
    rootsRow.style.display = 'flex';
    rootsRow.style.gap = '6px';
    rootsRow.style.flexWrap = 'wrap';
    const list = doc.createElement('div');
    list.style.maxHeight = '40vh';
    list.style.overflow = 'auto';
    const up = btn('上一级', 'btn btn-block');
    const pick = btn('选这个目录', 'btn btn-primary btn-block');
    const cancel = btn('取消', 'btn btn-block');
    async function load(p) {
      const r = await api('GET', '/api/dirs' + (p ? '?path=' + encodeURIComponent(p) : ''));
      pathIn.value = r.path || '';
      parentPath = r.parent || null;
      up.disabled = !parentPath;
      rootsRow.innerHTML = '';
      for (const root of (r.roots || [])) {
        const rb = btn(root, 'btn btn-ghost');
        rb.onclick = () => load(root).catch((e) => c.setMsg(e.message, true));
        rootsRow.appendChild(rb);
      }
      list.innerHTML = '';
      for (const d of (r.dirs || [])) {
        const b = btn(d.name, 'btn btn-block');
        b.style.textAlign = 'left';
        b.onclick = () => load(d.path).catch((e) => c.setMsg(e.message, true));
        list.appendChild(b);
      }
    }
    up.onclick = () => { if (parentPath) load(parentPath).catch((e) => c.setMsg(e.message, true)); };
    pick.onclick = () => { if (pathIn.value) { onPick(pathIn.value); c.close(); } };
    cancel.onclick = () => c.close();
    c.body.appendChild(pathIn);
    c.body.appendChild(rootsRow);
    c.body.appendChild(up);
    c.body.appendChild(list);
    c.body.appendChild(pick);
    c.body.appendChild(cancel);
    load(null).catch((e) => c.setMsg(e.message, true));
  }, true);
}

  openModal('新建工作', (c) => {
    const nameIn = textInput('工作名称（必填，会话落盘目录名）');
    if (w.name) nameIn.value = w.name;   // 草稿重开：把用户填过的带回来
    // 可手填：选择器只列本机盘符，网络共享（UNC）要自己写——虚拟机档接受，本机档由后端如实拒绝。
    const workspaceIn = textInput('默认（会话内 work/）；也可填绝对路径或 UNC：\\\\server\\share\\目录');
    if (w.workspace) workspaceIn.value = w.workspace;
    // 手填也算草稿：重开时带回来的是最后填的写法。
    workspaceIn.addEventListener('input', () => { w.workspace = workspaceIn.value; });
    const wsPick = btn('选择目录', 'btn btn-ghost');
    const wsDefault = btn('默认', 'btn btn-ghost');
    wsPick.onclick = () => pickWorkspaceDir((p) => { workspaceIn.value = p; w.workspace = p; });
    wsDefault.onclick = () => { workspaceIn.value = ''; w.workspace = ''; };
    const workspaceRow = doc.createElement('div');
    workspaceRow.style.display = 'flex';
    workspaceRow.style.gap = '8px';
    workspaceRow.appendChild(workspaceIn);
    workspaceRow.appendChild(wsPick);
    workspaceRow.appendChild(wsDefault);
    const wsHint = doc.createElement('div'); wsHint.className = 'wf-hint';
    wsHint.textContent = '本机档只接受本机目录（网络工作区会被拒绝，容器围栏访问不了私有网络）；' +
      '虚拟机档可填 UNC 共享下的目录，凭据用当前登录用户。';
    const workspaceWrap = doc.createElement('div');
    workspaceWrap.appendChild(workspaceRow);
    workspaceWrap.appendChild(wsHint);
    const modeSel = selectInput([
      { value: 'single', label: '单 agent（1 个或多个模块）' },
      { value: 'collab', label: '协作（多个 agent）' },
      { value: 'proxy', label: '代理（把决定权整块交给核心）' },
    ], 'single');
    // 形态说明随选择走（三种形态的差别就在这一句里说清）。
    const modeHint = doc.createElement('div'); modeHint.className = 'wf-hint';
    function modeHintText() {
      if (w.mode === 'proxy') {
        return '代理：没有名单要选——核心自己挑人、建子工作并代你决定（选这个形态就是授予全权）。\n' +
          '它的子会话不会把整份对话推给它；它靠观察与倒查消息决定下一步。想接管就按「停止」：相关的会话会一起停下。';
      }
      if (w.mode === 'collab') {
        return '协作：多个 agent 各自独立沙箱，先分权协商、再按任务链执行，由核心统一验收。';
      }
      return '单 agent：选 1 个模块 = 直连式；选多个 = 组合式。';
    }
    const partWrap = doc.createElement('div'); partWrap.className = 'wf-field';
    const modelWrap = doc.createElement('div'); modelWrap.className = 'wf-models';
    const taskIn = areaInput('本次需求（协作必填；也可写上，核心据此推荐）');
    if (w.task) taskIn.value = w.task;   // 草稿重开：同上
    // 上面那个「补齐虚拟机依赖」按钮在本作用域之外：把抄草稿的钩子交给它，点击时才读现值。
    snapshotDraft = () => { w.name = nameIn.value; w.task = taskIn.value; };
    // 需求那一格与推荐按钮都是**非代理形态**才问的：形态一换就收起来（见 renderParts）。
    const taskField = field('本次需求', taskIn);
    const recBtn = btn('让核心推荐', 'btn btn-block');
    const createBtn = btn('创建并开始', 'btn btn-primary btn-block');
    const cancelBtn = btn('取消', 'btn btn-block');
    // 没有可用模型就**不允许创建**：后端也会拒绝，这里先把原因说在前面。
    function createBlocked() {
      if (!state.models.length) {
        return '未登记任何模型，不能创建会话：先到「设置 → 模型与核心默认」登记供应商与模型';
      }
      if (w.mode === 'proxy') {
        return state.core ? '' : '代理会话必须设定核心默认模型：先到「设置 → 模型与核心默认」选定';
      }
      if (!w.agents.length) {
        return w.mode === 'single' ? '单 agent：先选或新建一个 agent' : '协作：先加入至少 1 个 agent';
      }
      const noModel = w.agents.find((a) => !a.model);
      if (noModel && !state.core) {
        return 'agent「' + noModel.name + '」没有默认模型，且核心默认未设定：给它指定一个模型，或先选定核心默认';
      }
      return '';
    }
    function syncCreateButton() {
      const why = createBlocked();
      createBtn.disabled = !!why;
      createBtn.title = why;
    }
    cancelBtn.onclick = closeModal;

    // 向导里的 agent 卡片：新建临时 / 从登记处加入；编辑已加入的卡片。
    function agentActions(single, hasAgent) {
      const row = doc.createElement('div'); row.className = 'wf-inline';
      const temp = btn(hasAgent ? '+ 更换为临时 agent' : '+ 新建临时 agent', 'btn');
      temp.onclick = () => {
        app.openTempAgentSheet({
          onAdd: (a) => { if (single) w.agents = [a]; else w.agents.push(a); renderAll(); },
          onSaveAsAgent: (a) => { if (single) w.agents = [a]; else w.agents.push(a); renderAll(); },
        });
      };
      const reg = btn(hasAgent ? '从登记处更换' : '从登记处加入', 'btn');
      reg.onclick = () => {
        app.openRegistryAgentPicker({
          single,
          taken: single ? [] : w.agents.map((a) => a.name),
          onPick: (a) => { if (single) w.agents = [a]; else w.agents.push(a); renderAll(); },
        });
      };
      row.appendChild(temp); row.appendChild(reg);
      if (!state.agents.length) {
        const hint = doc.createElement('div'); hint.className = 'wf-hint';
        hint.textContent = '（登记处还没有 agent：可以直接新建临时 agent）';
        row.appendChild(hint);
      }
      return row;
    }

    function saveCard(i) {
      const a = w.agents[i];
      if (!a || !a.transient) return;
      const name = a.name.trim();
      if (!name) { c.setMsg('agent 名字不能为空', true); return; }
      return (async () => {
        try {
          if (state.agents.some((x) => x.name === name)) {
            const ok = await confirmBox('同名 agent', '登记处已有「' + name + '」，保存会更新它。继续？', '更新');
            if (!ok) return;
          }
          await c.mutate('upsert_agent', {
            name,
            modules: a.modules.slice(),
            model: a.model || undefined,
            note: (a.note || '').trim(),
            chara: (a.chara || '').trim(),
          });
          a.name = name; a.transient = false; a.reuse = true;
          renderAll(); c.setMsg('已保存 agent：' + name);
        } catch (e) { c.setMsg(e.message, true); }
      })();
    }

    function cardHandlers(i) {
      return {
        onEdit: (a) => {
          if (a.transient) {
            app.openTempAgentSheet({
              agent: a,
              title: '编辑临时 agent',
              onAdd: (next) => { w.agents[i] = next; renderAll(); },
              onSaveAsAgent: (next) => { w.agents[i] = next; renderAll(); },
            });
          } else {
            // 登记处 agent 的本次 copy：名字与模块由登记处决定，人设 / 模型可就本工作调整。
            app.openTempAgentSheet({
              agent: a,
              title: '编辑本次 agent（不改登记处）',
              lockedName: true,
              lockedModules: true,
              allowSaveAsAgent: false,
              onAdd: (next) => { w.agents[i] = next; renderAll(); },
            });
          }
        },
        onSave: () => saveCard(i),
        onRemove: () => { w.agents.splice(i, 1); renderAll(); },
      };
    }

    function agentSection() {
      const box = doc.createElement('div'); box.className = 'form-col';
      if (w.mode === 'single') {
        if (w.agents.length) box.appendChild(app.agentCard(w.agents[0], cardHandlers(0)));
        else box.appendChild(emptyHint('（还没有选 agent：新建临时 agent，或从登记处加入）'));
        box.appendChild(agentActions(true, w.agents.length > 0));
        return box;
      }
      const list = doc.createElement('div'); list.className = 'agent-list';
      if (!w.agents.length) list.appendChild(emptyHint('（还没加入 agent）'));
      w.agents.forEach((a, i) => list.appendChild(app.agentCard(a, cardHandlers(i))));
      box.appendChild(list);
      box.appendChild(agentActions(false, false));
      return box;
    }

    function renderParts() {
      partWrap.innerHTML = '';
      modeHint.textContent = modeHintText();
      // 代理形态没有名单也没有模型要选：把该说的说清，把不该问的收起来。
      const proxy = w.mode === 'proxy';
      taskField.className = proxy ? 'wf-field hidden' : 'wf-field';
      recBtn.className = proxy ? 'btn btn-block hidden' : 'btn btn-block';
      if (proxy) {
        partWrap.appendChild(emptyHint(
          '（代理形态不需要选人）核心会用登记处里当前有效的 agent 与模块自己组队，' +
          '按你的目标建出一个或多个子工作，并代你回答它们的关卡。'
        ));
        return;
      }
      const lab = doc.createElement('div'); lab.className = 'wf-label';
      lab.textContent = w.mode === 'single'
        ? 'agent（单 agent：恰好 1 个；模块数不限）'
        : 'agent 名单（协作：可多个，各自独立沙箱）';
      partWrap.appendChild(lab);
      partWrap.appendChild(agentSection());
    }

    function renderModels() {
      modelWrap.innerHTML = '';
      if (w.mode === 'proxy') {
        modelWrap.appendChild(emptyHint('（代理会话走核心默认通道：模型按登记处的核心默认）'));
        if (!state.core) {
          modelWrap.appendChild(cfgHint('（核心默认模型未设定：代理会话不能创建；先到「设置 → 模型与核心默认」选定）', 'warn'));
        }
        return;
      }
      if (!state.models.length) {
        modelWrap.appendChild(cfgHint('（未登记任何模型：不能创建会话。先到「设置 → 模型与核心默认」登记。）', 'warn'));
        return;
      }
      if (!state.core && w.agents.some((a) => !a.model)) {
        modelWrap.appendChild(cfgHint('（核心默认未设定：没指定模型的 agent 不能创建；在卡片里给它选一个模型，或先选定核心默认）', 'warn'));
      }
    }

    function renderAll() { renderParts(); renderModels(); syncGuestGap(); syncCreateButton(); }

    modeSel.addEventListener('change', () => {
      w.mode = modeSel.value;
      // 代理形态默认显式选全可达（用户仍可在下拉里改；不再由后端硬编码）。
      if (w.mode === 'proxy') {
        w.permission = 'all_reach';
        profileSel.value = 'all_reach';
      }
      // 从协作切回单 agent：只保留第一张卡片（不静默替换成别的人）。
      if (w.mode === 'single' && w.agents.length > 1) w.agents = [w.agents[0]];
      renderAll();
    });

    recBtn.onclick = async () => {
      if (w.mode === 'proxy') { c.setMsg('代理形态不需要名单：核心自己挑人', true); return; }
      const task = taskIn.value.trim();
      if (!task) { c.setMsg('先写下本次需求，核心才能据此推荐', true); return; }
      c.setMsg('核心根据需求推荐中…');
      try {
        const r = await api('POST', '/api/suggest-models', { task, mode: w.mode });
        const rec = (r.agents || []).filter((a) => a && (a.name || (a.modules || []).length));
        if (!rec.length) { c.setMsg('核心未给出可用建议', true); return; }
        const asItem = (a) => {
          const reg = a.reuse === true ? state.agents.find((x) => x.name === a.name) : null;
          return {
            name: a.name || (a.modules && a.modules[0]) || 'agent',
            transient: a.reuse !== true,
            reuse: a.reuse === true,
            modules: ((reg && reg.modules) ? reg.modules : (a.modules || [])).slice(),
            model: a.model || (reg && reg.model) || '',
            chara: a.reuse === true && reg ? (reg.chara || a.chara || '') : (a.chara || ''),
            note: reg ? (reg.note || '') : '',
            why: a.why || null,
          };
        };
        const items = rec.map(asItem);
        w.agents = w.mode === 'single' ? [items[0]] : items;
        renderAll();
        c.setMsg(w.mode === 'single'
          ? '核心建议 agent「' + items[0].name + '」（见下方卡片；名字、人设、模块、模型都可改）'
          : '核心已给出建议（见下方卡片；每条都可改）');
      } catch (e) { c.setMsg(e.message, true); }
    };

    // 真正下发（重名时先经 choiceModal 让用户裁决，绝不用原生 confirm）
    // 要等：虚拟机档在这一步里起 guest（拉 QEMU + 等 agent，首次几十秒到几分钟）——按钮转圈 + 说清在等什么。
    async function submit(body) {
      const release = busyButton(createBtn, '正在创建…');
      cancelBtn.disabled = true;
      c.setMsg(w.tier === 'vm'
        ? '正在创建：虚拟机档要先起一台 guest（首次可能要几十秒到几分钟）；缺编译产物时还会先在 guest 里编一次…'
        : '正在创建：缺编译产物时先补编一次，编完就开工…');
      try {
        await startSession(body);
        closeModal();
      } catch (e) {
        c.setMsg('创建失败：' + e.message, true);
      } finally {
        release();
        cancelBtn.disabled = false;
      }
    }

    createBtn.onclick = async () => {
      const blocked = createBlocked();
      if (blocked) { c.setMsg(blocked, true); return; }
      const name = nameIn.value.trim();
      if (!name) { c.setMsg('工作名称必填', true); return; }
      const task = taskIn.value.trim();
      const vmGuest = (w.guest || '').trim();
      if (w.tier === 'vm' && !vmGuest) {
        c.setMsg('虚拟机档要先选一个虚拟机（虚拟根）：点上面的「补齐虚拟机依赖」装上，或换一个已装好的。', true);
        return;
      }
      if (w.tier === 'vm' && w.guestInfo && !w.guestInfo.ready) {
        const miss = (w.guestInfo.missing || []).join('、');
        c.setMsg('这个虚拟根还没装齐' + (miss ? '（缺 ' + miss + '）' : '') + '：先点「补齐虚拟机依赖…」装上再用。', true);
        return;
      }
      let agents;
      if (w.mode === 'proxy') {
        // 代理：没有名单、没有需求——选这个形态就是授予全权（档位由上面的选择显式落盘）。
        const proxyBody = { name, mode: 'proxy', agents: [], tier: w.tier, guest: w.tier === 'vm' ? vmGuest : null, permission: w.permission || 'all_reach' };
        if (workspaceIn.value.trim()) proxyBody.workspace = workspaceIn.value.trim();
        await submit(proxyBody);
        return;
      }
      if (w.mode === 'single') {
        if (!w.agents.length) { c.setMsg('单 agent：先选或新建一个 agent', true); return; }
        const a = w.agents[0];
        const mods = (a.modules || []).slice();
        const nm = (a.name || '').trim() || mods[0] || '';
        if (!nm) { c.setMsg('单 agent：给这个 agent 一个名字（或至少 1 个模块）', true); return; }
        agents = [{
          name: nm,
          transient: !!a.transient,
          modules: mods,
          model: a.model || undefined,
          chara: (a.chara || '').trim(),
        }];
      } else {
        if (!w.agents.length) { c.setMsg('请先加入至少 1 个 agent', true); return; }
        if (!task) { c.setMsg('协作模式必须填写本次需求', true); return; }
        agents = w.agents.map((a) => ({
          name: a.name.trim(),
          transient: !!a.transient,
          modules: a.modules.slice(),
          model: a.model || undefined,
          chara: (a.chara || '').trim(),
        }));
        if (agents.some((a) => !a.name)) { c.setMsg('agent 名字不能为空', true); return; }
      }
      const names = agents.map((a) => a.name);
      const dup = names.filter((n, i) => names.indexOf(n) !== i)[0];
      const body = { name, mode: w.mode, agents, tier: w.tier, permission: w.permission || 'workspace' };
      if (workspaceIn.value.trim()) body.workspace = workspaceIn.value.trim();
      if (w.tier === 'vm') body.guest = vmGuest;
      if (task) body.task = task;
      if (dup) {
        app.choiceModal('agent 重名', '本次工作里有同名 agent「' + dup + '」。', [
          ['按现名继续（后端自动加尾号 -2…）', 'btn btn-primary', () => submit(body)],
          ['回去改名', 'btn btn-ghost', () => {}],
        ]);
        return;
      }
      await submit(body);
    };

    c.body.appendChild(field('工作名称', nameIn));
    c.body.appendChild(field('工作区（可选）', workspaceWrap));
    c.body.appendChild(field('形态', modeSel));
    c.body.appendChild(modeHint);
    c.body.appendChild(tierField);
    c.body.appendChild(profileField);
    c.body.appendChild(pick.wrap);
    c.body.appendChild(pick.hint);
    c.body.appendChild(guestSetup);
    c.body.appendChild(gapHint);
    c.body.appendChild(partWrap);
    c.body.appendChild(modelWrap);
    c.body.appendChild(taskField);
    c.body.appendChild(recBtn);
    c.body.appendChild(createBtn);
    c.body.appendChild(cancelBtn);

    renderParts(); renderModels(); syncWizardTier(); syncCreateButton();
    // 别的标签页 / 设置在轮询后改了登记处：只换模型提示与创建按钮，不重建名字 / 需求输入框。
    c.onRegistryChange(() => { renderModels(); syncCreateButton(); });
  }, true);
}

/* ---------- 会话 ---------- */
async function startSession(body) {
  const r = await api('POST', actionUrl('create_session'), body);
  const sid = r.sid;
  const s = {
    sid, mode: body.mode, title: body.name || sid, run: 'active',
    lines: [], live: [], pending: null, sending: false,
    running: false, working: null, running_known: false,
    context: null, shell: null, work: null, workAt: 0, workDirty: false, workError: '',
    done: false, awaiting: null, fold: {}, scroll: {},
  };
  // 开场事实由 /api/sessions 那一刻就推给事件台了（回包只给头部序号），所以这里**合流**一次：
  // 盘上转录 + 事件台尾巴一次给全，水位也随之下定（此后的批次按水位筛）。
  s.hydrated = false;
  s.buffer = [];
  state.sessions.set(sid, s);
  hydrateHistory(s);
  // 开场事实不随回包走：它们已经在事件台上，长轮询会照 seq 补进来。
  setActive(sid);
  renderAll();
  await refreshState(); // 会话历史随建随现
  return s;
}

function setActive(sid) {
  atClose(); // 换会话 = 换一份文件清单，菜单先收起来
  state.activeSid = sid;
  // 换会话先清掉上一份根（免得拿别人的根去缩）；缓存里有就同步先缩，避免先长后短的闪烁。
  shortPathRoots = null;
  const cached = app.filesCache.get(sid);
  if (cached) shortPathRoots = buildRootList(cached.roots);
  renderTabs(); renderStream(true); renderGate(state.sessions.get(sid));
  const s = state.sessions.get(sid);
  if (s) { syncComposer(s); loadWork(sid); }
  if (!s || !s.readonly) $('#input').focus();
  if (!cached) loadPathRoots(sid); // 没缓存就去拉一次（失败静默），拿到后补一次重渲染
}

/* 把 roots 应用到当前活动会话；返回是否变化（变了才值得重渲染）。 */
function applyPathRoots(sid, data) {
  if (state.activeSid !== sid) return false;
  const next = buildRootList(data && data.roots);
  const same = JSON.stringify(next) === JSON.stringify(shortPathRoots);
  shortPathRoots = next;
  return !same;
}

/* 会话激活时就把路径根拉到手（复用 @ 菜单那份缓存）；失败静默，不影响会话本身。 */
async function loadPathRoots(sid) {
  const data = await atLoad(sid);
  if (!data) return;
  if (applyPathRoots(sid, data)) renderStream(true); // 根到手 → 重渲染一次，长路径立刻变短
}

function activeSession() { return state.sessions.get(state.activeSid); }

/* 回档/重置：清空当前会话视图，并按服务端给的**活动视图**整体重建。
 * 回档会改写盘上转录；事件台上的旧事实不随文件回退，所以服务端另推一条 `rewound`
 * 快照。这里用同一段逻辑处理“动作回包”和“其它标签页收到的重置事件”。 */
function applySnapshot(s, events) {
  s.lines = [];
  s.live = [];
  s.fold = {};
  s.scroll = {};
  s.forceFull = true;
  s.pending = null;
  s.done = false;
  s.context = null; // 回档后的用量由快照里的 usage 事件重建；没有就显示"还没有调用"
  s.workDirty = true; // 回档可能也物化了共享区：状态栏重新拉一次提交
  s.readonly = false; // 回档后的会话是活动会话（历史回放被用户明确回档/继续）
  for (const ev of (events || [])) absorb(s, ev);
}

/* ---------- 事件吸收：统一把 SessionEvent 变成行 ---------- */
function absorb(s, ev) {
  switch (ev.type) {
    case 'notice': s.lines.push({ cls: 'sys', who: '', text: ev.text }); break;
    // 上下文用量：不是转录行、不进模型上下文；只更新状态栏（持久事件，刷新后仍在）。
    case 'usage':
      s.context = {
        prompt_tokens: ev.prompt_tokens || 0,
        completion_tokens: ev.completion_tokens || 0,
        total_tokens: ev.total_tokens || 0,
        window: ev.window || 0,
        exact: ev.exact === true,
        cache_read: typeof ev.cache_read === 'number' ? ev.cache_read : null,
        cache_creation: typeof ev.cache_creation === 'number' ? ev.cache_creation : null,
      };
      return true;
    // 留档回档：折叠分界。内容保留在文件里，可恢复。
    case 'rewind':
      if (ev.mode !== 'archive') break;
      s.lines.push({
        cls: 'sys rewind',
        who: '',
        text: ev.label
          ? '[留档] ' + ev.label + '（折叠内容仍在文件里，可恢复）'
          : '[留档] 以下内容已折叠（保留在文件里，可恢复）',
        mark: ev.mark,
      });
      break;
    // 回档成功：服务端给的是**回档后的活动视图**；清空旧视图整体重建。
    // 刷新/多端靠这条事件同步，而不是把回档前的旧事件当实时尾巴再拼一遍。
    case 'rewound':
      applySnapshot(s, ev.events || []);
      break;
    // 任务链的进展：节点开工/回报/验收/交付——主会话也要看得到，不必点进子会话。
    case 'node_started':
      // 节点是执行进度，不是转录内容：独立的卡片（有状态 / 执行人 / 进入子会话），不复用系统消息。
      if (ev.sid) state.nodeBySid.set(ev.sid, { node: ev.node || '', assignee: ev.assignee || '' });
      s.lines.push({
        cls: 'node',
        node: { id: ev.node || '', sid: ev.sid || '', assignee: ev.assignee || '' },
        state: 'started',
        report: '',
        rework: 0,
        who: '',
        text: '',
      });
      break;
    // 节点回报 = **它把活交回来了**：找到同一节点的卡标完成，不另起一行。
    // 失败的节点由核心的验收结论另行如实说明（这里只显示"交回"这个事实，不替验收下判）。
    case 'report': {
      const id = ev.id || '';
      let hit = null;
      for (let i = s.lines.length - 1; i >= 0; i -= 1) {
        const l = s.lines[i];
        if (l.cls === 'node' && l.node && l.node.id === id) { hit = l; break; }
      }
      if (hit) {
        hit.state = 'done';
        hit.report = ev.text || '';
        hit.rework = ev.rework || 0;
      } else {
        s.lines.push({
          cls: 'node',
          node: { id, sid: '', assignee: '' },
          state: 'done',
          report: ev.text || '',
          rework: ev.rework || 0,
          who: '',
          text: '',
        });
      }
      s.forceFull = true; // 卡在原位更新：报告不频繁，整帧重建比维护节点映射更稳
      break;
    }
    case 'transcript':
      // 服务端权威转录：每行带会话内稳定 id（回档按 id 定位）。
      // 权威行到达：撤掉乐观回显与流式块，改用服务端的行；带 tool 的行渲染成工具卡片。
      const hadPending = s.lines.some((x) => x.pending);
      s.lines = s.lines.filter((x) => !x.pending);
      if (hadPending) s.forceFull = true; // 乐观回显被权威行替换：整帧重建，别留下没有 id 的旧节点
      for (const l of ev.lines) {
        // 身份与样式**全读结构化字段**（种类 / 说话人 / 动词），不再从正文里抠标签（见 lineParts）。
        const parts = lineParts(l);
        for (const p of parts) { p.id = l.id; if (l.reasoning) p.reasoning = l.reasoning; }
        s.lines.push(...parts);
      }
      s.live = []; // 权威行整体替换掉流式块
      break;
    case 'delta': {
      // 流式块列表（短暂，不落盘）：一次发言可能有**多轮模型调用**（工具循环），
      // 所以按到达顺序排块——kind='start' 表示又一轮开始：封存上一块并推入新块，绝不丢弃。
      if (ev.kind === 'start' || !s.live.length || s.live[s.live.length - 1].kind !== 'msg') {
        // 每个流式轮一个**唯一键**：V{序号} 若按数组下标复用，上一轮展开的思维链会污染下一轮。
        s.liveSeq = (s.liveSeq || 0) + 1;
        s.live.push({ kind: 'msg', speaker: ev.speaker || '', segments: [], key: 'V' + s.liveSeq });
      }
      if (ev.kind === 'start') return true;
      const segs = s.live[s.live.length - 1].segments;
      const last = segs[segs.length - 1];
      if (last && last.kind === ev.kind) last.text += ev.text;
      else segs.push({ kind: ev.kind, text: ev.text });
      return true; // 短暂流式事件：只走增量渲染，不整帧重建
    }
    case 'working':
      // **权威运行态**：核心开始问某个 agent = 忙（带名字），agent=null = 这一回合收尾了。
      // 它不进转录（短暂事件）：用户看到的是占位动画与按钮切换，不是一条消息。
      // 事件即真相：agent 有名字 = 忙，null = 这一回合收尾（此后不再听快照的）。
      s.running = !!ev.agent;
      s.working = ev.agent || null;
      s.running_known = true;
      if (!ev.agent) {
        // 回合收尾却没有权威行（停止 / 错误）：未定稿的分片不得继续闪光标或冒充转录。
        s.live = [];
        return false;
      }
      return true;
    case 'tool_call':
      // 工具调用发生在轮与轮之间：按到达顺序插进流式块里（module 为空 = 内置 read/write）。
      s.live.push({ kind: 'tool', tool: ev });
      // agent 提交会改共享区 head：让状态栏 / 提交弹窗下次刷新时跟上。
      if (ev.name === 'work_commit') s.workDirty = true;
      // shell 跑完了：运行槽只经 /api/state 下发，立刻拉一次——chip 与打开的卡片不用等 3 秒轮询。
      if (ev.name === 'shell') needState = true;
      return true; // 同上：卡片节点只 append，绝不重建
    case 'discussion_done':
      if (ev.over_cap) s.lines.push({ cls: 'sys', who: '', text: '讨论超轮次上限，进入裁决。' });
      break;
    case 'plan': s.lines.push({ cls: 'plan', who: '核心整理', text: ev.text }); break;
    case 'review':
      if (!ev.items || ev.items.length === 0) {
        s.lines.push({ cls: 'bad', who: '验收', text: '清单解析失败，原文：\n' + ev.raw });
      } else {
        const txt = ev.items.map((i) => '[' + i.status.toUpperCase() + '] ' + i.item + (i.note ? ' —— ' + i.note : '')).join('\n');
        s.lines.push({ cls: 'review', who: '验收清单', text: txt });
      }
      break;
    case 'delivery':
      s.lines.push(ev.ok ? { cls: 'ok', who: '交付', text: '全部通过，交付用户。' }
        : { cls: 'bad', who: '裁决', text: '返工超限仍未通过，交用户裁决。' });
      break;
    case 'decision_card':
      // **裁决卡**：与快照里的 pending 是同一个事实，只是到达得更快。
      // waiting 里是排在队首之后还在等的几张（前面还排着几条由它如实显示）。
      s.pending = ev;
      return false; // 门要整帧重画
    case 'decision_answer':
      // 一次回答的记录：这一关不再挂着（下一关的卡会随后自己推来）。
      s.pending = null;
      return false; // 门要整帧重画
    case 'decision_void':
      // **整队作废**（用户按停止 / 会话关闭 = 拒绝）：队列里没答的卡一律作废，界面撤下这张卡。
      s.pending = null;
      return false; // 门要整帧重画
    case 'ended':
      // 整场工作结束：运行态也是"没在跑"（事件给的事实，不靠快照）。
      s.done = true;
      s.live = [];
      s.running = false;
      s.working = null;
      s.running_known = true;
      break;
  }
  return false; // 定稿事件：需要整帧重建
}

/* 吸收一批事件：全是短暂流式事件 → 增量渲染；出现任何定稿事件 → 整帧重建。
 * 返回 'live' | 'full' | 'none'。 */
function absorbEvents(s, events) {
  let live = false, full = false;
  for (const ev of (events || [])) {
    if (absorb(s, ev) === true) live = true;
    else full = true;
  }
  return full ? 'full' : (live ? 'live' : 'none');
}

/* [id:tag] text → 行对象（呈现即上下文：原样收录，标签做样式） */
/// 一行 → 渲染对象：**全读结构化字段**（种类 / 说话人 / 动词 / 正文），不从正文里抠标签。
/// 服务端在落行时就把身份写进字段（见 core/events.rs 的 LineView）——
/// 这里只做"种类 → 样式"的映射：系统行、用户行、轮次分隔行、模型发言、工具卡片。
function lineParts(l) {
  const text = l.line || '';
  const speaker = l.speaker || '';
  const verb = l.verb || '';
  const kind = l.kind || '';
  const reasoning = l.reasoning || null;
  // 工具行：按调用视图渲染成卡片（说话人取自视图）。
  if (l.tool || kind === 'tool') {
    // 工具行同样要有身份（卡片左上角标出是谁在调工具）——身份是这一行的，不是正文的附属。
    return [{
      cls: 'tool',
      tool: l.tool || null,
      reasoning: reasoning,
      who: (l.tool && l.tool.speaker) || speaker,
      text: '',
      speaker: (l.tool && l.tool.speaker) || speaker,
      reply: l.reply || (l.tool && l.tool.reply) || 0,
      raw: l.raw || '',
    }];
  }
  // 压缩记录：转录里的一等行（有 id，可删除/留档/恢复）；摘要 + 从哪一行起保留原文。
  if (kind === 'compacted') {
    const upTo = (l.payload && l.payload.up_to) || 0;
    return [{
      cls: 'sys compacted',
      who: '',
      text: '[压缩] 第 ' + upTo + ' 行之前已压成摘要；此后原样保留（旧内容仍可查看）：\n' + text,
      reply: l.reply || 0,
    }];
  }
  // 内容类系统提示：转录里的一等行（有 id，可回档/折叠），界面仍按系统行画。
  if (kind === 'notice') return [{ cls: 'sys notice', who: speaker, text: text }];
  // 系统行：系统注入的提醒/边界（没有说话人），或核心自己的行（代拟…）。
  if (kind === 'system' || l.system) return [{ cls: 'sys system', who: speaker, text: text }];
  // 讨论的轮次分隔行：标签是"轮次 + 号"（号在正文里，结构化）。
  if (kind === 'round') return [{ cls: 'sys system', who: '', text: '[' + speaker + ' ' + text + ']' }];
  // 用户说的行：标签在字段里（需求/开始/撤回/名单…），界面上就是"用户 + 那句话"。
  if (kind === 'user') return [{ cls: 'user', who: '用户', text: text }];
  // 模型发言：动词决定样式；"降级"是结构化信号（不在正文里找说明文案）。
  const cls = verb === 'agree' ? 'ok' : verb === 'leave' ? 'sys' : verb === 'ask' ? 'plan' : 'line';
  const deg = l.degraded === true;
  return [{
    cls: deg ? 'sys' : cls,
    who: speaker + (verb ? ' · ' + verb : '') + (deg ? ' · 信封缺失' : ''),
    text: text,
    speaker: speaker,
    verb: verb,
    degraded: deg,
    reply: l.reply || 0,
    raw: l.raw || '',
  }];
}

/* ---------- 渲染 ---------- */
function renderAll() {
  const s = activeSession();
  renderTabs(); renderStream(); renderGate(s); syncComposer(s);
  if (s && s.workDirty) { s.workDirty = false; loadWork(s.sid, true); }
}

/* 只读历史视图：不渲染输入区；只留「启用会话」。启用是后端动作，前端只渲染结果。 */
function renderReadonly(s) {
  const bar = $('#readonly-bar');
  if (!bar) return;
  bar.innerHTML = '';
  if (!s || !s.readonly) { bar.classList.add('hidden'); return; }
  bar.classList.remove('hidden');
  const note = doc.createElement('span');
  note.className = 'readonly-note';
  if (s.workspace_missing) {
    note.textContent = '工作区已不存在：恢复目录或删除会话后才能启用（当前只能查看记录）';
    bar.appendChild(note);
    return;
  }
  note.textContent = s.run === 'closed' ? '历史回放（只读）· 已关闭' : '历史回放（只读）：启用后才能发送消息';
  bar.appendChild(note);
  if (s.run === 'closed') return;
  const b = doc.createElement('button');
  b.className = 'btn btn-primary';
  b.textContent = s.activating ? '正在启用…' : '启用会话';
  b.disabled = !!s.activating;
  b.onclick = activateFlow;
  bar.appendChild(b);
}

/* 输入区与上下文继续的显隐：只按后端给的视图（readonly / continue_action / running）渲染。 */
function syncComposer(s) {
  const composer = $('#composer');
  if (composer) {
    if (s && s.readonly) composer.classList.add('hidden');
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

/* ---------- 会话状态栏：上下文用量 + 共享区提交 ----------
 * 数据源：/api/state 的 SessionView.context（精确/估算）、持久 usage 事件、/api/sessions/{sid}/work。
 * 约束：这里只做呈现与只读拉取；回滚走统一动作分发（动作 id restore_work_commit）。 */
function fmtInt(n) {
  const v = Number(n || 0);
  return Number.isFinite(v) ? v.toLocaleString('zh-CN') : '0';
}
function ctxPercent(ctx) {
  if (!ctx || !ctx.window) return null;
  return (Number(ctx.total_tokens || 0) / Number(ctx.window)) * 100;
}
function ctxChipClass(percent) {
  const at = Number((state.settings && state.settings.compact_at_percent) || 0);
  if (percent == null) return '';
  if (percent >= 90) return 'danger';
  if (at > 0 && percent >= at) return 'warn';
  return '';
}
/// shell 是不是仍在跑：回放派生的快照没有 finished_at，但它不是"正在跑"。
function shellRunning(sh) { return !!sh && !sh.historical && !sh.finished_at; }
/// shell 回执状态里要跟退出码分开显示的几类：超时 / 停止 / 信号；exit 与旧数据（null）留空走退出码。
function shellAbnormal(sh) {
  if (!sh) return '';
  if (sh.status === 'timeout') return '超时';
  if (sh.status === 'stopped') return '停止';
  if (sh.status === 'signal') return '中止（信号）';
  return '';
}
/// shell 卡片的「状态」行：超时 / 停止 / 信号从「失败」里分出来；exit 与旧数据按成败。
function shellStatusLabel(sh, running) {
  if (running) return '运行中…';
  if (sh.status === 'timeout') return '超时';
  if (sh.status === 'stopped') return '停止';
  if (sh.status === 'signal') return '被信号结束';
  return sh.ok ? '成功' : '失败';
}
/// shell 的耗时（秒）：实时槽按墙钟差；回放派生用回执里的耗时行。
function shellSecs(sh) {
  if (sh.elapsed_secs != null) return Math.max(0, Number(sh.elapsed_secs) || 0);
  const end = sh.finished_at || Math.floor(Date.now() / 1000);
  return Math.max(0, end - (sh.started_at || 0));
}
function fmtDuration(sec) {
  const s = Math.max(0, Math.floor(Number(sec) || 0));
  if (s < 60) return s + ' 秒';
  const m = Math.floor(s / 60);
  const r = s % 60;
  return m + ' 分' + (r ? r + ' 秒' : '');
}
function timeAgo(ts) {
  const t = Number(ts || 0);
  if (!t) return '';
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - t);
  if (diff < 60) return '刚刚';
  if (diff < 3600) return Math.floor(diff / 60) + ' 分钟前';
  if (diff < 86400) return Math.floor(diff / 3600) + ' 小时前';
  if (diff < 30 * 86400) return Math.floor(diff / 86400) + ' 天前';
  return fmtDateTime(t);
}
function fmtDateTime(ts) {
  const t = Number(ts || 0);
  if (!t) return '—';
  try {
    return new Date(t * 1000).toLocaleString('zh-CN', { hour12: false });
  } catch (e) { return String(t); }
}
function statusPrintOf(s) {
  const c = s && s.context;
  const w = s && s.work;
  return JSON.stringify([
    s && s.sid, c && c.prompt_tokens, c && c.completion_tokens, c && c.total_tokens,
    c && c.window, c && c.exact, w && w.head && w.head.id, w && w.total, w && w.error,
    s && s.workError, s && s.workAt ? 1 : 0,
    (state.settings && state.settings.compact_at_percent) || 0,
    (state.settings && state.settings.compact_keep_percent) || 0,
    (state.settings && state.settings.compact_model) || '',
    s && s.mode,
    s && s.permission_profile, s && s.permission_label, s && s.permission_narrowed ? 1 : 0,
    s && s.permission_parent,
    s && s.thinking ? [s.thinking.level, s.thinking.default_level, s.thinking.label, (s.thinking.options || []).length] : 0,
    s && s.context_freshness ? 1 : 0,
    s && s.shell ? [
      s.shell.command, s.shell.scope, s.shell.cwd, s.shell.started_at, s.shell.finished_at,
      s.shell.elapsed_secs, s.shell.ok, s.shell.exit_code, (s.shell.output || '').length,
      s.shell.historical ? 1 : 0,
      shellRunning(s.shell) ? Math.floor(Date.now() / 1000) : 0,
    ] : 0,
  ]);
}

/* 思考强度的兜底目录：后端 /api/state 的 settings.thinking 是权威；桩或加载失败时仍能画出四档。 */
const DEFAULT_THINKING_LEVELS = [
  { id: 'off' },
  { id: 'low' },
  { id: 'high' },
  { id: 'max' },
];
function thinkingLevels() {
  const t = state.settings && state.settings.thinking;
  const levels = t && Array.isArray(t.levels) ? t.levels : [];
  return levels.length ? levels : DEFAULT_THINKING_LEVELS;
}
function thinkingLabel(id) {
  return String(id || '');
}

const PERMISSION_ORDER = ['view', 'workspace', 'external', 'all_reach'];
const PERMISSION_LABELS = {
  view: '仅可查看',
  workspace: '工作区内修改',
  external: '外部可达（逐次确认）',
  all_reach: '全可达（自动放行）',
};

/* 权限档位小卡片：只列四档 + 一个“更多设置”。子会话禁用比父档更宽的选项；
 * 生成中禁用全部（后端也会拒绝，这里先给出原因，免得白点一下）。 */
function openPermissionMenu(chip, s) {
  const current = s.permission_profile || 'workspace';
  const parent = s.permission_parent || null;
  const parentIdx = parent ? PERMISSION_ORDER.indexOf(parent) : PERMISSION_ORDER.length - 1;
  const items = PERMISSION_ORDER.map((id, i) => {
    const disabled = s.running || (!!parent && i > parentIdx);
    const reason = s.running
      ? '会话正在生成：先停止再切换权限'
      : (!!parent && i > parentIdx ? '子会话只能收窄：要放宽请改主会话' : '');
    return {
      icon: id === current ? 'check' : (id === 'all_reach' ? 'alert' : 'shield'),
      label: PERMISSION_LABELS[id] + (id === current ? '（当前）' : ''),
      disabled,
      reason,
      onClick: () => { if (id !== current) switchPermission(s.sid, id); },
    };
  });
  if (s.mode !== 'proxy') {
    items.push({ sep: true });
    items.push({ icon: 'gear', label: '更多权限设置…', onClick: () => openConfig(s.sid) });
  }
  ui.menu(chip, items);
}

/* 思考强度小卡片：工作根一个档；只列后端给的已映射档，没选过档时直接勾共同默认档。 */
function openThinkingMenu(chip, s) {
  const th = s.thinking || { level: null, default_level: null, label: '未设置', options: [], note: '' };
  const current = th.level || th.default_level || '';
  const items = [];
  // 没有统一默认档时（各模型默认不同 / 都没设默认），保留一个跟随默认的状态项。
  if (!th.default_level) {
    items.push({
      icon: !th.level ? 'check' : 'dot',
      label: '跟随默认' + (!th.level ? '（当前）' : ''),
      disabled: !!s.running,
      reason: s.running ? '会话正在生成：先停止再切换思考强度' : '',
      onClick: () => { if (th.level) switchThinking(s.sid, ''); },
    });
  }
  for (const o of (th.options || [])) {
    const isCurrent = current === o.id;
    items.push({
      icon: isCurrent ? 'check' : 'dot',
      label: o.label + (isCurrent ? '（当前）' : ''),
      disabled: !!s.running,
      reason: s.running ? '会话正在生成：先停止再切换思考强度' : '',
      onClick: () => {
        if (isCurrent) return;
        switchThinking(s.sid, o.id === th.default_level ? '' : o.id);
      },
    });
  }
  ui.menu(chip, items);
}

/* 切换思考强度：只写工作根 meta，下一次生成生效；后端拒绝时原样显示原因。 */
async function switchThinking(sid, level) {
  const s = state.sessions.get(sid);
  if (!s) return;
  if (s.running) { ui.toast('warn', '会话正在生成：先停止再切换思考强度'); return; }
  try {
    await api('POST', actionUrl('set_thinking'), { session_id: sid, level });
    await refreshState();
    ui.toast('ok', '思考强度：' + (level ? thinkingLabel(level) : '跟随模型默认'));
  } catch (e) {
    ui.toast('err', e.message || String(e));
  }
}

/* 快速切档只改 profile（并回到该档的 reach / decision 默认）；共享区范围保留，要改这些走“更多权限设置”。 */
async function switchPermission(sid, profile) {
  const s = state.sessions.get(sid);
  if (!s) return;
  if (s.running) { ui.toast('warn', '会话正在生成：先停止再切换权限'); return; }
  try {
    await api('POST', actionUrl('set_permission_profile'), { session_id: sid, profile });
    await refreshState();
    ui.toast('ok', '权限档位：' + (PERMISSION_LABELS[profile] || profile));
  } catch (e) {
    ui.toast('err', e.message || String(e));
  }
}

function syncStatusbar(s) {
  const box = $('#session-status');
  if (!box) return;
  if (!s || state.activeSid !== s.sid) {
    box.classList.add('hidden');
    box.innerHTML = '';
    return;
  }
  const print = statusPrintOf(s);
  if (s.statusPrint === print) {
    box.classList.remove('hidden');
    paintShellCard();
    return;
  }
  s.statusPrint = print;
  box.innerHTML = '';
  box.classList.remove('hidden');
  const inner = doc.createElement('div');
  inner.className = 'session-status-inner';

  // 权限档位：proxy 与子会话都必须看得见它实际用的档。
  if (s.permission_label) {
    const permChip = ui.statusChip({
      icon: 'shield',
      text: s.permission_label + (s.permission_narrowed ? '（收窄）' : ''),
      label: '权限档位',
      title: '当前权限档位：' + s.permission_label + '（点开可切换；子会话只能收窄）',
    });
    permChip.onclick = () => openPermissionMenu(permChip, s);
    inner.appendChild(permChip);
  }

  // 思考强度：工作根一个档；只列这组模型都登记了的档。
  if (s.thinking) {
    const th = s.thinking;
    const thChip = ui.statusChip({
      icon: 'cpu',
      text: '思考 ' + (th.label || '未设置'),
      label: '思考强度',
      title: (th.note ? th.note : '当前思考强度：' + (th.label || '未设置') + '（点开可切换；子会话与工作根共享）'),
      cls: th.note ? 'warn' : '',
    });
    thChip.onclick = () => openThinkingMenu(thChip, s);
    inner.appendChild(thChip);
  }

  // shell：运行中 / 最近一次的结果；点开看命令、scope、cwd、退出码与回执预览。
  if (s.shell) {
    const sh = s.shell;
    const running = shellRunning(sh);
    const dur = fmtDuration(shellSecs(sh));
    // 历史回放没有墙钟时间：耗时只取回执里的「耗时」行，没留下就不写。
    const hist = sh.historical ? '（历史）' : '';
    const since = sh.historical && sh.elapsed_secs == null ? '' : ' · ' + dur;
    const abnormal = shellAbnormal(sh);
    let text;
    if (running) text = 'shell 运行中 · ' + dur;
    else if (abnormal) text = 'shell ' + abnormal + hist + since;
    else if (sh.historical) {
      const code = sh.exit_code == null ? (sh.ok ? '完成' : '中止') : '退出码 ' + sh.exit_code;
      text = 'shell ' + code + hist + since;
    } else if (sh.exit_code == null) text = 'shell ' + (sh.ok ? '完成' : '中止') + since;
    else text = 'shell 退出码 ' + sh.exit_code + since;
    const shellChip = ui.statusChip({
      icon: 'terminal',
      text,
      label: 'shell',
      title: (running ? '正在执行：' : '最近一次：') + sh.command,
      cls: running ? 'warn' : (sh.ok ? '' : 'danger'),
    });
    shellChip.onclick = () => openShellCard(s, shellChip);
    inner.appendChild(shellChip);
  }

  // 协作主会话没有一条持续发给模型的对话：不显示上下文 chip（不伪造数字）。
  if (s.mode !== 'collab') {
    const c = s.context;
    const pct = ctxPercent(c);
    const label = c
      ? (c.exact ? '' : '≈') + fmtInt(c.total_tokens) + ' tok · ' + (pct == null ? '—' : pct.toFixed(1) + '%')
      : '上下文 —';
    const tip = c
      ? '上下文：' + (c.exact ? '供应商 usage（精确）' : '按字符估算') +
        '；' + fmtInt(c.total_tokens) + ' / ' + fmtInt(c.window) + ' tokens'
      : '已经有模型调用后显示上下文用量';
    const chip = ui.statusChip({
      icon: 'chart',
      text: label,
      label: '上下文用量',
      title: tip,
      cls: ctxChipClass(pct),
      onClick: () => openContextCard(s, chip),
    });
    inner.appendChild(chip);
  }

  // 工具：先放手动压缩；所有会话都给入口，协作主会话的项置灰并写明原因。
  const toolsChip = ui.statusChip({
    icon: 'gear',
    text: '工具',
    label: '工具',
    title: '压缩上下文（/compact）',
    onClick: () => openToolsMenu(toolsChip, s),
  });
  inner.appendChild(toolsChip);

  // 共享区提交：head + 相对时间；点击看详情 / 全部提交。
  const work = s.work;
  const head = work && work.head;
  let workLabel = 'work …';
  if (s.workError) workLabel = 'work 不可用';
  else if (work) workLabel = head ? '#' + head.id + ' work · ' + timeAgo(head.time) : 'work 空';
  const workChip = ui.statusChip({
    icon: 'git',
    text: workLabel,
    label: '共享区提交',
    title: head ? ('#' + head.id + ' ' + head.author + '：' + head.message) : '共享区提交历史',
    cls: s.workError ? 'warn' : '',
    onClick: () => openWorkCard(s, workChip),
  });
  inner.appendChild(workChip);
  box.appendChild(inner);
  paintShellCard();
}

async function loadWork(sid, force) {
  const s = state.sessions.get(sid);
  if (!s) return;
  if (s.workLoading) return;
  if (!force && s.workAt && Date.now() - s.workAt < 10000) return;
  s.workLoading = true;
  s.workDirty = false;
  try {
    const r = await api('GET', '/api/sessions/' + encodeURIComponent(sid) + '/work?limit=20');
    if (state.sessions.get(sid) !== s) return;
    s.work = r;
    s.workError = '';
  } catch (e) {
    if (state.sessions.get(sid) !== s) return;
    s.workError = e.message;
  } finally {
    s.workLoading = false;
    s.workAt = Date.now();
    if (state.activeSid === sid) syncStatusbar(s);
  }
}

function popoverBox(title, close) {
  const box = doc.createElement('div');
  const head = doc.createElement('div');
  head.className = 'popover-head';
  const h = doc.createElement('div');
  h.className = 'popover-title';
  h.textContent = title || '';
  const x = doc.createElement('button');
  x.type = 'button';
  x.className = 'popover-close';
  x.setAttribute('aria-label', '关闭');
  x.textContent = '✕';
  x.onclick = close || (() => ui.closePopover());
  head.appendChild(h); head.appendChild(x);
  box.appendChild(head);
  return box;
}
function popoverRow(k, v) {
  const row = doc.createElement('div');
  row.className = 'popover-row';
  const a = doc.createElement('span'); a.className = 'k'; a.textContent = k;
  const b = doc.createElement('span'); b.className = 'v'; b.textContent = v;
  row.appendChild(a); row.appendChild(b);
  return row;
}
function popoverNote(text, cls) {
  const n = doc.createElement('div');
  n.className = 'popover-note' + (cls ? ' ' + cls : '');
  n.textContent = text;
  return n;
}

/* 最近一次 shell 的卡片：内容可重画——运行中每秒刷新耗时，结束后由状态刷新重画终态。 */
let shellCardOpen = null; // { sid, box, body, dur, sig }

function shellCardSig(sh) {
  return [sh.command, sh.scope, sh.cwd, sh.started_at, sh.finished_at, sh.elapsed_secs,
    sh.exit_code, sh.status, sh.ok, sh.historical ? 1 : 0, (sh.output || '').length].join('\u0001');
}

/// 画一张 shell 卡片的内容；返回「耗时」那一格，运行中只改它。
function fillShellCard(body, sh) {
  body.innerHTML = '';
  const running = shellRunning(sh);
  if (sh.command) {
    const cmd = doc.createElement('pre');
    cmd.className = 'tool-pre';
    cmd.textContent = sh.command;
    body.appendChild(cmd);
  }
  body.appendChild(popoverRow('scope', sh.scope === 'external'
    ? 'external（共享区 / 模块目录 / 主机只读根 / 网络）'
    : 'sandbox（只碰私有沙箱）'));
  body.appendChild(popoverRow('cwd', sh.cwd || '—'));
  body.appendChild(popoverRow('状态', shellStatusLabel(sh, running)));
  body.appendChild(popoverRow('退出码', sh.exit_code == null ? '—' : String(sh.exit_code)));
  const durRow = popoverRow('耗时', sh.historical && sh.elapsed_secs == null
    ? '—（历史记录没有留下）'
    : fmtDuration(shellSecs(sh)));
  body.appendChild(durRow);
  if (sh.output) {
    const out = doc.createElement('pre');
    out.className = 'tool-pre';
    out.textContent = sh.output;
    body.appendChild(out);
  }
  body.appendChild(popoverNote(sh.historical
    ? '来自转录回放：耗时与 cwd 取自当时的回执；完整回执在转录的 shell 工具行。'
    : '完整回执在转录的 shell 工具行；这里只保留最近一次。'));
  return durRow.children[1];
}

function paintShellCard() {
  const o = shellCardOpen;
  if (!o) return;
  const s = state.sessions.get(o.sid);
  const sh = s && s.shell;
  if (!sh) {
    if (o.sig !== 'none') {
      o.sig = 'none'; o.dur = null;
      o.body.innerHTML = '';
      o.body.appendChild(popoverNote('这条会话还没有执行过 shell。'));
      if (typeof o.place === 'function') o.place();
    }
    return;
  }
  const sig = shellCardSig(sh);
  if (sig !== o.sig) {
    o.sig = sig;
    o.dur = fillShellCard(o.body, sh);
    // 内容变高/变矮：重新夹一次位置，别让新增的回执把卡片顶出视口底部。
    if (typeof o.place === 'function') o.place();
  } else if (o.dur && shellRunning(sh)) {
    o.dur.textContent = fmtDuration(shellSecs(sh));
  }
}

function openShellCard(s, anchor) {
  const box = popoverBox('shell');
  const body = doc.createElement('div');
  box.appendChild(body);
  const timer = typeof setInterval === 'function' ? setInterval(paintShellCard, 1000) : null;
  const pop = ui.popover(anchor, box, {
    label: 'shell',
    onClose: () => {
      if (timer != null && typeof clearInterval === 'function') clearInterval(timer);
      if (shellCardOpen && shellCardOpen.box === box) shellCardOpen = null;
    },
  });
  shellCardOpen = { sid: s.sid, box: box, body: body, dur: null, sig: '', place: pop.place };
  paintShellCard();
}

function openContextCard(s, anchor) {
  const box = popoverBox('上下文用量');
  const c = s.context;
  if (!c) {
    box.appendChild(popoverNote('还没有模型调用。下一次调用后这里会显示精确用量；拿不到供应商 usage 时按字符估算并标注。'));
    ui.popover(anchor, box, { label: '上下文用量' });
    return;
  }
  const pct = ctxPercent(c);
  const at = Number((state.settings && state.settings.compact_at_percent) || 0);
  if (pct != null) {
    const bar = doc.createElement('div');
    bar.className = 'ctx-bar';
    const fill = doc.createElement('div');
    fill.className = 'ctx-fill' + (pct >= 90 ? ' danger' : (at > 0 && pct >= at ? ' warn' : ''));
    fill.style.width = Math.max(0, Math.min(100, pct)) + '%';
    bar.appendChild(fill);
    if (at > 0 && at <= 100) {
      const mark = doc.createElement('span');
      mark.className = 'ctx-mark';
      mark.style.left = at + '%';
      mark.title = '自动压缩阈值 ' + at + '%';
      bar.appendChild(mark);
    }
    box.appendChild(bar);
  }
  box.appendChild(popoverRow('已用', fmtInt(c.total_tokens) + ' tokens'));
  box.appendChild(popoverRow('窗口', fmtInt(c.window) + ' tokens'));
  box.appendChild(popoverRow('占用', pct == null ? '—' : pct.toFixed(1) + '%'));
  box.appendChild(popoverRow('上次输入 / 输出', fmtInt(c.prompt_tokens) + ' / ' + fmtInt(c.completion_tokens)));
  if (c.cache_read != null || c.cache_creation != null) {
    box.appendChild(popoverRow(
      '缓存读 / 写',
      (c.cache_read == null ? '—' : fmtInt(c.cache_read)) + ' / ' +
        (c.cache_creation == null ? '—' : fmtInt(c.cache_creation)),
    ));
  }
  box.appendChild(popoverRow('自动压缩', at > 0 ? at + '%' : '已关闭'));
  const keep = Number((state.settings && state.settings.compact_keep_percent) || 0);
  box.appendChild(popoverRow('压缩保留', keep > 0 ? '至少 ' + keep + '% 窗口' : '只留摘要'));
  box.appendChild(
    popoverRow('总结模型', (state.settings && state.settings.compact_model) || '默认（会话同模型）'),
  );
  const badge = doc.createElement('span');
  badge.className = 'badge-soft ' + (c.exact ? 'ok' : 'warn');
  badge.textContent = c.exact ? '供应商 usage（精确）' : '按字符估算';
  box.appendChild(badge);
  box.appendChild(popoverNote(c.exact
    ? '数字来自供应商返回的 usage；工具结果回灌的瞬间可能略小于下一请求。'
    : '供应商没回 usage：按对话字符 ÷ 4 估算，只作参考。'));
  ui.popover(anchor, box, { label: '上下文用量' });
}

/* 手动压缩：协作主会话没有一条持续对话，由成员会话各自压。 */
function compactNow(s) {
  if (!s) return;
  if (s.mode === 'collab') {
    notice('协作会话不直接压缩', '协作主会话没有一条持续发给模型的对话；打开成员会话再用工具菜单压缩。', 'info');
    return;
  }
  act('compact');
}

/* 状态栏「工具」菜单：先放手动压缩；以后再加其它工具。 */
function openToolsMenu(chip, s) {
  const keep = Number((state.settings && state.settings.compact_keep_percent) || 0);
  const model = (state.settings && state.settings.compact_model) || '默认（会话同模型）';
  const blocked = s.mode === 'collab'
    ? '协作主会话没有持续对话；成员会话各自压缩'
    : (isBusy(s) ? '生成中，先停止或等这一轮结束' : (s.readonly ? '历史回放为只读' : ''));
  ui.menu(chip, [
    {
      label: '压缩上下文（/compact）',
      icon: 'box',
      disabled: !!blocked,
      reason: blocked,
      onClick: () => compactNow(s),
    },
    {
      label: '保留 ' + (keep > 0 ? '至少 ' + keep + '% 窗口' : '只留摘要') + ' · 总结模型 ' + model,
      disabled: true,
    },
    { sep: true },
    {
      label: '过期内容换成占位符' + (s.context_freshness ? '（已开）' : '（关）'),
      icon: s.context_freshness ? 'check' : 'box',
      disabled: !!blocked,
      reason: blocked,
      onClick: () => switchContextFreshness(s.sid, !s.context_freshness),
    },
    {
      label: '开：读过的文件被改后，那条工具结果换成占位符（省上下文；每次发送要多读一遍对象库，且会失效部分缓存）',
      disabled: true,
    },
  ]);
}

/* 切换过期内容替换：写工作根 meta，整棵子树共享；后端拒绝时原样显示原因。 */
async function switchContextFreshness(sid, on) {
  const s = state.sessions.get(sid);
  if (!s) return;
  if (isBusy(s)) { ui.toast('warn', '会话正在生成：先停止再切换'); return; }
  try {
    await api('POST', actionUrl('set_context_freshness'), { session_id: sid, on: !!on });
    await refreshState();
    ui.toast('ok', on ? '过期内容：已改为占位符' : '过期内容：保持原样');
  } catch (e) {
    ui.toast('err', e.message || String(e));
  }
}

function openWorkCard(s, anchor) {
  const box = popoverBox('共享区');
  if (s.workError) {
    box.appendChild(popoverNote('提交信息读取失败：' + s.workError));
  } else if (!s.work) {
    box.appendChild(popoverNote('正在读取提交历史…'));
    if (!s.workLoading) {
      loadWork(s.sid, true).then(() => {
        if (state.activeSid === s.sid && !s.work) return;
        if (state.activeSid === s.sid) { ui.closePopover(); openWorkCard(s, anchor); }
      });
    }
  } else if (!s.work.head) {
    box.appendChild(popoverNote('还没有提交。agent 的 work_commit 或你的上传会出现在这里。'));
  } else {
    const h = s.work.head;
    box.appendChild(popoverRow('当前 head', '#' + h.id));
    box.appendChild(popoverRow('作者 / 时间', h.author + ' · ' + timeAgo(h.time)));
    const msg = doc.createElement('div');
    msg.className = 'commit-msg';
    msg.textContent = h.message;
    box.appendChild(msg);
    const rows = (s.work.commits || []).slice(0, 5);
    if (rows.length) {
      const list = doc.createElement('div');
      list.className = 'work-history-list';
      list.style.maxHeight = '180px';
      for (const c of rows) {
        const row = doc.createElement('button');
        row.type = 'button';
        row.className = 'work-row' + (c.on_head ? ' current' : '');
        const rh = doc.createElement('div'); rh.className = 'work-row-head';
        const id = doc.createElement('span'); id.className = 'work-id'; id.textContent = '#' + c.id;
        const meta = doc.createElement('span'); meta.className = 'work-meta';
        meta.textContent = (c.on_head ? '当前线 · ' : '已离开当前线 · ') + c.author + ' · ' + timeAgo(c.time);
        rh.appendChild(id); rh.appendChild(meta);
        const m = doc.createElement('div'); m.className = 'work-msg'; m.textContent = c.message;
        row.appendChild(rh); row.appendChild(m);
        row.onclick = () => { ui.closePopover(); openWorkHistory(s, c.id); };
        list.appendChild(row);
      }
      box.appendChild(list);
    }
  }
  const foot = doc.createElement('div');
  foot.className = 'popover-foot';
  const all = btn('全部提交', 'btn btn-ghost');
  all.onclick = () => { ui.closePopover(); openWorkHistory(s); };
  foot.appendChild(all);
  box.appendChild(foot);
  ui.popover(anchor, box, { label: '共享区提交' });
}

function openWorkHistory(s, selected) {
  loadWork(s.sid, true);
  openModal('提交历史 · ' + s.sid, (c) => {
    const wrap = doc.createElement('div');
    wrap.className = 'work-history';
    const list = doc.createElement('div');
    list.className = 'work-history-list';
    const detail = doc.createElement('div');
    detail.className = 'work-detail';
    wrap.appendChild(list); wrap.appendChild(detail);
    c.body.appendChild(wrap);
    c.setMsg('回滚是非破坏性的：目标之后的提交仍保留，可再次回滚；只改共享区，不动各 agent 沙箱。');
    let page = null;
    let selectedId = selected || null;
    let loading = false;

    const paintDetail = async (id) => {
      if (!id) {
        detail.innerHTML = '<div class="work-empty">选一条提交看详情。</div>';
        return;
      }
      detail.innerHTML = '<div class="work-empty">读取中…</div>';
      let d;
      try {
        d = await api('GET', '/api/sessions/' + encodeURIComponent(s.sid) + '/work/' + encodeURIComponent(id));
      } catch (e) {
        detail.innerHTML = '';
        detail.appendChild(popoverNote('读取失败：' + e.message, 'err'));
        return;
      }
      detail.innerHTML = '';
      const head = doc.createElement('div');
      head.className = 'work-detail-head';
      const title = doc.createElement('div');
      title.className = 'work-detail-title';
      title.textContent = '#' + d.id + (d.parent ? ' ← #' + d.parent : '');
      const badge = doc.createElement('span');
      badge.className = 'badge-soft' + (s.work && s.work.head && s.work.head.id === d.id ? ' ok' : '');
      badge.textContent = s.work && s.work.head && s.work.head.id === d.id ? '当前 head' : '历史提交';
      head.appendChild(title); head.appendChild(badge);
      detail.appendChild(head);
      detail.appendChild(popoverRow('作者 / 时间', d.author + ' · ' + fmtDateTime(d.time)));
      const msg = doc.createElement('div');
      msg.className = 'commit-msg';
      msg.textContent = d.message;
      detail.appendChild(msg);
      const chHead = doc.createElement('div');
      chHead.className = 'work-meta';
      chHead.textContent = '改动 ' + d.changes.length + ' 处' + (d.changes_truncated ? '（已截断）' : '');
      detail.appendChild(chHead);
      const changes = doc.createElement('div');
      changes.className = 'change-list';
      for (const ch of d.changes) {
        const row = doc.createElement('button');
        row.type = 'button';
        row.className = 'change-row';
        const kind = doc.createElement('span');
        kind.className = 'change-kind ' + ch.kind;
        kind.textContent = ch.kind === 'add' ? '新增' : ch.kind === 'modify' ? '修改' : '删除';
        const path = doc.createElement('span');
        path.className = 'change-path';
        path.textContent = ch.path;
        const size = doc.createElement('span');
        size.className = 'change-size';
        size.textContent = ch.size + ' B';
        row.appendChild(kind); row.appendChild(path); row.appendChild(size);
        if (ch.kind !== 'delete') {
          row.title = '预览该提交里的这个文件';
          row.onclick = () => previewCommitFile(detail, s, d.id, ch.path);
        } else {
          row.disabled = true;
          row.title = '该提交删除了这个文件';
        }
        changes.appendChild(row);
      }
      detail.appendChild(changes);
      const foot = doc.createElement('div');
      foot.className = 'popover-foot';
      const restore = btn('回滚到此处', 'btn btn-primary');
      const blocked = s.readonly ? '历史回放只读：先启用会话' : (isBusy(s) ? '会话正在生成：先停止再回滚' : '');
      if (blocked) { restore.disabled = true; restore.title = blocked; }
      restore.onclick = () => restoreCommit(c, s, d);
      foot.appendChild(restore);
      detail.appendChild(foot);
    };

    const paintList = (append) => {
      if (!page) return;
      if (!append) list.innerHTML = '';
      for (const cm of page.commits) {
        if (list.querySelector('[data-commit="' + cm.id + '"]')) continue;
        const row = doc.createElement('button');
        row.type = 'button';
        row.dataset.commit = String(cm.id);
        row.className = 'work-row' + (cm.on_head ? ' current' : '');
        const rh = doc.createElement('div'); rh.className = 'work-row-head';
        const id = doc.createElement('span'); id.className = 'work-id'; id.textContent = '#' + cm.id;
        const meta = doc.createElement('span'); meta.className = 'work-meta';
        meta.textContent = (cm.on_head ? '当前线 · ' : '已离开 · ') + cm.author + ' · ' + timeAgo(cm.time) + ' · ' + cm.changes + ' 处';
        rh.appendChild(id); rh.appendChild(meta);
        const m = doc.createElement('div'); m.className = 'work-msg'; m.textContent = cm.message;
        row.appendChild(rh); row.appendChild(m);
        row.onclick = () => { selectedId = cm.id; paintDetail(cm.id); };
        list.appendChild(row);
      }
    };
    const loadPage = async (before) => {
      if (loading) return;
      loading = true;
      try {
        const q = '/api/sessions/' + encodeURIComponent(s.sid) + '/work?limit=50' + (before ? '&before=' + before : '');
        const r = await api('GET', q);
        page = r;
        s.work = r;
        s.workAt = Date.now();
        syncStatusbar(s);
        paintList(!!before);
        if (r.truncated) {
          const old = list.querySelector('.work-more');
          if (old) old.remove();
          const more = btn('加载更早', 'btn btn-ghost work-more');
          more.onclick = () => loadPage(page.commits[page.commits.length - 1].id);
          list.appendChild(more);
        }
        if (!selectedId) selectedId = r.head ? r.head.id : (r.commits[0] && r.commits[0].id);
        paintDetail(selectedId);
      } catch (e) {
        c.setMsg('读取提交历史失败：' + e.message, true);
      } finally {
        loading = false;
      }
    };
    loadPage(null);
  }, true);
}

async function previewCommitFile(holder, s, commit, path) {
  const old = holder.querySelector('.file-preview');
  if (old) old.remove();
  const box = doc.createElement('div');
  box.className = 'file-preview';
  box.appendChild(popoverNote('读取 ' + path + ' …'));
  holder.appendChild(box);
  try {
    const r = await api('GET', '/api/sessions/' + encodeURIComponent(s.sid) + '/work/' +
      encodeURIComponent(commit) + '?path=' + encodeURIComponent(path));
    box.innerHTML = '';
    box.appendChild(popoverRow('文件', r.path));
    box.appendChild(popoverRow('大小 / 类型', r.size + ' B · ' + (r.media_type || '未知')));
    if (r.too_large) box.appendChild(popoverNote('超过共享区单文件上限：只给元信息，不预览。'));
    else if (r.text == null) box.appendChild(popoverNote('二进制 / 非 UTF-8：只给元信息。'));
    else {
      const pre = doc.createElement('pre');
      pre.textContent = r.text + (r.truncated ? '\n…（预览已截断，共 ' + r.size + ' B）' : '');
      box.appendChild(pre);
    }
  } catch (e) {
    box.innerHTML = '';
    box.appendChild(popoverNote('读取失败：' + e.message, 'err'));
  }
}

async function restoreCommit(modal, s, d) {
  if (s.readonly) { notice('历史回放只读', '先启用会话，再回滚共享区。', 'info'); return; }
  if (isBusy(s)) { notice('会话正在生成', '先停止再回滚，避免与正在提交的 agent 交错。', 'info'); return; }
  let head = s.work && s.work.head ? s.work.head.id : 0;
  const text = '共享区将从 head #' + head + ' 回滚到提交 #' + d.id + '（' + d.author + '：' + d.message + '）。\n\n' +
    '这是非破坏性回滚：目标之后的提交仍保留在历史里，可再次回滚；只改共享区主副本，不动各 agent 沙箱，后续拉取可能遇到冲突。';
  if (!(await confirmBox('回滚共享区', text, '回滚'))) return;
  try {
    await api('POST', actionUrl('restore_work_commit'), {
      session_id: s.sid, commit: d.id, expect_head: head,
    });
    s.workAt = 0; s.workDirty = false;
    await loadWork(s.sid, true);
    ui.toast('ok', '已回滚到提交 #' + d.id);
    if (modal && modal.close) modal.close();
  } catch (e) {
    if (/被外部改动/.test(e.message) &&
        await confirmBox('覆盖工作区外部改动', e.message + '\n\n确认后会覆盖这些路径当前的内容；建议先 work_sync，保留外部修改。', '仍要覆盖')) {
      try {
        await api('POST', actionUrl('restore_work_commit'), {
          session_id: s.sid, commit: d.id, expect_head: head, force: true,
        });
        s.workAt = 0; s.workDirty = false;
        await loadWork(s.sid, true);
        ui.toast('ok', '已确认覆盖并回滚到提交 #' + d.id);
        if (modal && modal.close) modal.close();
      } catch (e2) {
        if (modal && modal.setMsg) modal.setMsg('回滚失败：' + e2.message, true);
        else notice('回滚失败', e2.message, 'err');
      }
      return;
    }
    if (modal && modal.setMsg) modal.setMsg('回滚失败：' + e.message, true);
    else notice('回滚失败', e.message, 'err');
    s.workAt = 0;
    loadWork(s.sid, true);
  }
}

/* ---------- 会话标签：主 → 子 → 子子，按活跃路径逐层展开 ----------
 * 第 1 行是根（工作）；第 N 行是上一行活跃会话的孩子。只显示活跃路径上的层。
 * 孩子来源 = history（含没打开过的，弱化显示）+ 已打开的视图；点弱化的才去加载。 */
function sessionParent(name) {
  const s = state.sessions.get(name);
  if (s && s.parent) return s.parent;
  const h = (state.history || []).find((x) => x.name === name);
  return (h && h.parent) || null;
}

function knownChildren(parent) {
  const names = [];
  const seen = new Set();
  for (const h of (state.history || [])) {
    if ((h.parent || null) === parent && !seen.has(h.name)) { seen.add(h.name); names.push(h.name); }
  }
  for (const s of state.sessions.values()) {
    const p = s.parent || sessionParent(s.sid);
    if ((p || null) === parent && !seen.has(s.sid)) { seen.add(s.sid); names.push(s.sid); }
  }
  return names;
}

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

/* 转录 DOM：定稿容器 + 流式容器（都挂在 #stream 下）。
 * 定稿容器只在"整帧重建"时重建；流式容器在 delta / tool_call 路径上只做**增量 append**，
 * 已有节点绝不重建——<details> 的开关、<pre> 的滚动条、正在拖的滚动位置都不会被打断。 */
let domOwner = null;   // 两个容器当前属于哪个会话
let doneBox = null;
let liveBox = null;
let typingNode = null;

/// 打字指示器只有一个，靠 class 显隐；不参与流的 append 顺序。
function syncTyping(s) {
  if (!typingNode) return;
  const live = s.live || [];
  const hasLive = live.some((b) => b.kind === 'tool' || (b.segments && b.segments.length));
  const busy = isBusy(s);
  typingNode.textContent = s.working ? '正在工作：' + s.working : '正在工作…';
  typingNode.className = busy && !hasLive ? 'typing' : 'typing hidden';
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

/// 只有本来就在底部才自动跟随；用户往上滚时保持原位置（流式刷新不抢滚动条）。
function nearBottom(box) { return box.scrollHeight - box.scrollTop - box.clientHeight < 80; }

/// 回档按钮：回到这一行（留档或删除，点开后再选）。图标 + 原生 title（长说明不会被滚动裁掉）。
function rewindButton(id) {
  const b = ui.iconButton({
    icon: 'undo',
    label: '回档到此处',
    tip: '',
    cls: 'line-act',
    onClick: () => rewindTo(id),
  });
  b.title = '回到这一行：留档（可恢复）或删除（不可恢复）';
  return b;
}

/// 建立两个容器 + 打字指示器（整帧重建时调用）。
function buildStreamBoxes(sid) {
  const box = $('#stream');
  box.innerHTML = '';
  doneBox = doc.createElement('div'); doneBox.className = 'stream-done';
  liveBox = doc.createElement('div'); liveBox.className = 'stream-live';
  typingNode = doc.createElement('div'); typingNode.className = 'typing hidden'; typingNode.textContent = 'agent 工作中';
  box.appendChild(doneBox); box.appendChild(liveBox); box.appendChild(typingNode);
  domOwner = sid;
}

/// **唯一的建块函数**：一行（渲染部件）→ 一个节点。身份在上、思维链次之、正文按 kind；工具行走卡片。
/// 定稿行与实时块都从这里出——"两处各建一次"正是身份时有时无、空块漏出来的来源。
function lineBlock(p, s, key, live) {
  if (p.tool) {
    const card = toolCard(p.tool, s, key, p.who || p.speaker);
    if (p.reasoning && state.settings.show_reasoning) card.appendChild(reasoningBlock(p.reasoning, s, key));
    return { node: card, txt: null, cot: null };
  }
  const cls = p.cls || 'line';
  const el = doc.createElement('div');
  el.className = 'line ' + cls + (live ? ' streaming' : '');
  if (p.who) {
    const w = doc.createElement('span'); w.className = 'who'; w.textContent = p.who; el.appendChild(w);
  }
  // 思维链在回答之上（先想后说）；默认折叠，展开状态由 fold store 记住（键 = 行/块的稳定键）。
  let cot = null;
  if (p.reasoning && state.settings.show_reasoning) {
    cot = reasoningBlock(p.reasoning, s, key);
    el.appendChild(cot);
  }
  let txt = null;
  if (String(p.text == null ? '' : p.text).length) {
    appendBody(el, cls, p.text);
    txt = el.children[el.children.length - 1] || null;
  }
  return { node: el, txt: txt, cot: cot };
}

/// 定稿行渲染到容器里（每次全量重建这个容器；折叠与 <pre> 滚动状态由 store 恢复）。
/// 会话级说明（代理身份 / 已停止 / 已关闭）：放转录最上方，**不冒充任何一方的发言**。
/// 它只说这条会话是什么、还该不该被驱动——正文一律来自转录本身。
function sessionNote(s) {
  if (!s) return null;
  const bits = [];
  if (s.mode === 'proxy') {
    bits.push('代理模式：核心代你挑人、建子工作并回答它们的关卡——这就是全权；' +
      '按「停止」会让相关会话一起停下。');
  }
  if (s.run === 'stopped') bits.push('这条会话已停止：不会再被派发或唤醒；启用后按会话状态继续。');
  if (s.run === 'closed') bits.push('这条会话已关闭：终态，不会再被派发或唤醒。');
  if (s.readonly) bits.push('历史回放为只读：点「启用会话」后才能发送消息。');
  if (!bits.length) return null;
  const d = doc.createElement('div');
  d.className = 'agent-note';
  d.textContent = bits.join('\n');
  return d;
}

/* ---------- 定稿渲染：按 reply 归成「一轮」，只追加新行 ----------
 * 一轮 = 首次模型回复的 id；同 reply 的正文 / 思维链 / 工具行合成一个气泡。
 * 增量：只处理 s.lines 里还没渲染过的尾部；同一 reply 的后续行只重建这一轮（折叠状态按 key 恢复）。
 * 整帧重建只发生在：切会话、重置/回档、思维链开关、会话说明变化、渲染计数倒退。 */
let doneState = { owner: null, count: 0, show: null, note: '', last: null };

function turnWorthy(p) {
  return (p.cls === 'line' || p.cls === 'ok' || p.cls === 'tool') && typeof p.id === 'number';
}

function turnKey(group) {
  return group.reply ? 'R' + group.reply : 'K' + group.id;
}

function turnText(parts) {
  return parts
    .filter((p) => p.cls !== 'tool')
    .map((p) => String(p.text == null ? '' : p.text))
    .filter((t) => t.trim())
    .join('\n\n');
}

function turnReasoning(parts) {
  // reasoning 可能挂在第一条 tool 行上（模型只发工具调用、没有正文）；不能按 cls 过滤掉。
  const seen = new Set();
  const out = [];
  for (const p of parts) {
    const r = p.reasoning;
    if (!r || seen.has(r)) continue;
    seen.add(r);
    out.push(r);
  }
  return out.join('\n\n');
}

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
    w.onclick = () => withdrawAgree(head.speaker);
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
    go.onclick = () => openPath(n.sid);
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
  if (l.cls === 'node') { doneBox.appendChild(nodeCard(l, s)); return; }
  const bodyless = !String(l.text == null ? '' : l.text).trim();
  if (bodyless && !l.reasoning) return;
  const built = lineBlock(l, s, 'L' + l.id, false);
  if (typeof l.id === 'number') built.node.appendChild(rewindButton(l.id));
  if (typeof l.mark === 'number') {
    const rb = ui.iconButton({ icon: 'undo', label: '恢复留档', tip: '', cls: 'line-act' });
    rb.title = '恢复到这次留档之前（标记及其后的内容会被删除）';
    rb.onclick = () => restoreMark(l.mark);
    built.node.appendChild(rb);
  }
  doneBox.appendChild(built.node);
}

function appendFinalLine(l, s) {
  if (!turnWorthy(l)) {
    doneState.last = null;
    appendFinalRow(l, s);
    return;
  }
  const reply = l.reply || 0;
  const last = doneState.last;
  if (last && reply && last.reply === reply) {
    // 同一次回复的后续行（典型是工具行）：只重建这一轮，节点替换，折叠状态按 key 恢复。
    const parts = last.parts.concat([l]);
    const fresh = buildTurn({ kind: 'turn', reply, parts, id: parts[0].id }, s);
    if (!fresh) return;
    if (last.node.parentNode && last.node.parentNode.replaceChild) {
      last.node.parentNode.replaceChild(fresh.node, last.node);
    } else {
      doneBox.removeChild(last.node);
      doneBox.appendChild(fresh.node);
    }
    doneState.last = fresh;
    return;
  }
  const turn = buildTurn({ kind: 'turn', reply, parts: [l], id: l.id }, s);
  if (!turn) return;
  doneBox.appendChild(turn.node);
  doneState.last = turn;
}

function renderDone(s, full) {
  if (!doneBox) return;
  const note = sessionNote(s);
  const noteText = note ? note.textContent : '';
  const show = !!state.settings.show_reasoning;
  const bad = full || !!s.forceFull || doneState.owner !== s.sid || doneState.count > s.lines.length
    || doneState.show !== show || doneState.note !== noteText;
  if (bad) {
    s.forceFull = false;
    doneBox.innerHTML = '';
    doneState = { owner: s.sid, count: 0, show, note: noteText, last: null };
    if (note) doneBox.appendChild(note);
    for (const l of s.lines) appendFinalLine(l, s);
    doneState.count = s.lines.length;
  } else if (doneState.count < s.lines.length) {
    for (const l of s.lines.slice(doneState.count)) appendFinalLine(l, s);
    doneState.count = s.lines.length;
  }
  flushScroll(s);
}

/// 流式块：只做增量——块不存在就建节点，内容只在既有节点上更新，绝不重建。
/// **一个块最多一个思维链折叠 + 一段正文**（与定稿后的行同形）：模型会把"思维链/正文"交替吐出来，
/// 若按到达片段各建节点，流式期间就会冒出两个「思维链」、正文被切成好几段。
/// 流式 tool 块的键 = 已有权威 tool 行数 + 它在 live 里的 tool 块序号 → 与定稿后的权威行同键。
function liveTurnNode(owner) {
  const el = doc.createElement('article');
  el.className = 'turn live';
  const head = doc.createElement('div');
  head.className = 'turn-head';
  const who = doc.createElement('span');
  who.className = 'turn-who';
  who.textContent = (owner && owner.speaker) || (owner && owner.tool && owner.tool.speaker) || '';
  head.appendChild(who);
  el.appendChild(head);
  const body = doc.createElement('div');
  body.className = 'turn-body';
  el.appendChild(body);
  return { node: el, body, cot: null, textWrap: null, text: null, textValue: '', tools: null, toolList: null };
}

function updateLiveGroup(g, gi, s, showCot, streaming) {
  const ui = g.ui;
  const segs = (g.msg && g.msg.segments) || [];
  let cot = '';
  let txt = '';
  for (const seg of segs) {
    const t = String(seg.text == null ? '' : seg.text);
    if (!t) continue;
    if (seg.kind === 'reasoning') cot += t;
    else txt += t;
  }
  if (showCot && cot) {
    if (!ui.cot) {
      const node = reasoningBlock(cot, s, (g.msg && g.msg.key) || ('V' + gi));
      if (ui.textWrap) ui.body.insertBefore(node, ui.textWrap);
      else ui.body.appendChild(node);
      ui.cot = { node, text: cot };
    } else if (ui.cot.text !== cot) {
      const body = ui.cot.node.children && ui.cot.node.children[1];
      if (body) body.textContent = cot;
      ui.cot.text = cot;
    }
  }
  if (txt) {
    if (!ui.textWrap) {
      ui.textWrap = doc.createElement('div');
      ui.textWrap.className = 'turn-text';
      ui.body.appendChild(ui.textWrap);
      ui.text = mdNode(txt);
      ui.textWrap.appendChild(ui.text);
      ui.textValue = txt;
    } else if (ui.textValue !== txt) {
      if (typeof markdownToHtml === 'function') ui.text.innerHTML = shortPath(markdownToHtml(txt));
      else ui.text.textContent = txt;
      ui.textValue = txt;
    }
  }
  if (g.tools.length) {
    if (!ui.tools) {
      ui.tools = doc.createElement('details');
      ui.tools.className = 'fold turn-tools';
      const sum = doc.createElement('summary');
      ui.tools.appendChild(sum);
      ui.toolList = doc.createElement('div');
      ui.toolList.className = 'tool-list';
      ui.tools.appendChild(ui.toolList);
      ui.body.appendChild(ui.tools);
    }
    const failed = g.tools.filter((tb) => tb.tool && !tb.tool.ok).length;
    ui.tools.children[0].textContent = '工具调用 · ' + g.tools.length + (failed ? '（' + failed + ' 失败）' : '');
    for (const tb of g.tools) {
      if (tb._node) continue;
      tb._node = toolCard(tb.tool, s, 'LT' + gi + '-' + g.tools.indexOf(tb), '');
      ui.toolList.appendChild(tb._node);
    }
  }
  const visible = (showCot && !!cot) || !!txt || g.tools.length > 0;
  ui.node.className = 'turn live' + (visible ? '' : ' empty') + (streaming && visible ? ' streaming' : '');
}

/// 流式块按“一次回复”分组：消息 + 随后的工具调用合成同一个临时轮，定稿后形状不变。
function renderLive(s, full) {
  if (!liveBox) { renderStream(true); return; }
  const live = s.live || [];
  if (full) {
    liveBox.innerHTML = '';
    for (const b of live) { b._node = null; b._ui = null; }
  }
  const groups = [];
  for (const b of live) {
    if (b.kind === 'msg') groups.push({ msg: b, tools: [] });
    else if (b.kind === 'tool') {
      if (!groups.length) groups.push({ msg: null, tools: [] });
      groups[groups.length - 1].tools.push(b);
    }
  }
  const showCot = !!state.settings.show_reasoning;
  let lastMsg = -1;
  groups.forEach((g, gi) => { if (g.msg) lastMsg = gi; });
  groups.forEach((g, gi) => {
    const owner = g.msg || g.tools[0];
    if (!owner) return;
    if (!owner._ui) {
      owner._ui = liveTurnNode(owner);
      liveBox.appendChild(owner._ui.node);
    }
    g.ui = owner._ui;
    if (g.msg) g.msg._node = owner._ui.node;
    updateLiveGroup(g, gi, s, showCot, gi === lastMsg);
  });
  flushScroll(s);
}

/// 整帧重建：容器与全部行都重建（折叠 / <pre> 滚动由 store 恢复）。
function renderStream(force) {
  const box = $('#stream');
  const prevTop = box.scrollTop;
  const stick = force === true || nearBottom(box);
  const s = activeSession();
  if (!s) {
    doneBox = null; liveBox = null; typingNode = null; domOwner = null;
    doneState = { owner: null, count: 0, show: null, note: '', last: null };
    box.innerHTML = '<div class="empty"><div class="logo">☉</div>' +
      '<div class="tip">点左上角「+ 新建工作」开始：单 agent（直连式 / 组合式）· 协作</div>' +
      '<div class="hint">PC：双栏布局　移动端：左上角 ☰ 打开侧栏</div></div>';
    return;
  }
  const full = force === true || domOwner !== s.sid || !doneBox || !liveBox;
  if (full) {
    // 切会话 / 重置 / 开关变化：容器与全部行重建（折叠与 <pre> 滚动由 store 恢复）。
    doneBox = null; liveBox = null; typingNode = null; domOwner = null;
    box.innerHTML = '';
    buildStreamBoxes(s.sid);
    doneState.owner = null;
    renderDone(s, true);
    renderLive(s, true);
  } else {
    // 追加路径：只处理新到的定稿行；流式块只在"这一轮已经收尾"时撤下。
    renderDone(s, false);
    if (!s.live || !s.live.length) {
      if (liveBox) liveBox.innerHTML = '';
      for (const b of (s.live || [])) { b._node = null; b._cot = null; b._txt = null; }
    } else {
      renderLive(s, false);
    }
  }
  syncTyping(s);
  box.scrollTop = stick ? box.scrollHeight : prevTop;
  syncSendButton(s);
}

/* 流式增量按帧合并：一帧最多一次 DOM 写，高频分片不会把主线程塞满。 */
let liveTickScheduled = false;
function scheduleLiveTick() {
  if (liveTickScheduled) return;
  liveTickScheduled = true;
  const run = () => {
    liveTickScheduled = false;
    renderLiveTick(activeSession());
  };
  if (typeof g.requestAnimationFrame === 'function') g.requestAnimationFrame(run);
  else g.setTimeout(run, 0);
}

/// 流式增量帧：只 append 新的流式节点，绝不重建已有节点。
function renderLiveTick(s) {
  if (!s) return;
  if (!liveBox || domOwner !== s.sid) { renderStream(true); return; }
  const box = $('#stream');
  const prevTop = box.scrollTop;
  const stick = nearBottom(box);
  renderLive(s, false);
  syncTyping(s);
  box.scrollTop = stick ? box.scrollHeight : prevTop;
  syncSendButton(s);
  syncStatusbar(s);
  if (s.workDirty) { s.workDirty = false; loadWork(s.sid, true); }
}

/* 裁决门：**按后端给的消息与选项渲染**（前端不认识业务含义，也不按 kind 猜按钮）。
   改需求**不是**门——它是会话级按钮（见 syncSendButton / updateTaskFlow）。 */
function renderGate(s) {
  const gate = $('#gate');
  gate.innerHTML = '';
  if (!s || s.done || s.readonly) return;
  const p = s.pending;
  if (isBusy(s) && !p) return;
  if (s.awaiting === 'task' && !isBusy(s)) {
    gate.appendChild(gateCard('请提交本次协作需求：', [
      ['提交', async () => { const v = takeInput(); if (v) await act('task', v); }],
    ]));
    return;
  }
  // **卡片照画**：工具级确认发生在生成中（生成正停下来等这一答），核心关卡的卡也一样——
  // 前端只认后端给的四格（信封 / 消息 / 选项），不按 kind 猜按钮、也不在忙时把它藏起来。
  if (!p) return;
  gate.appendChild(renderCard(p, s));
}

/// 裁决卡：信封 / 消息（标题 / 正文 / 详情）/ 选项——四格都由后端给，前端照画（**扁平**，见契约）。
function renderCard(p, s) {
  const busy = !!(s && s.sending);
  const msg = p.message || {};
  const el = doc.createElement('div');
  el.className = 'gate-card';
  const who = doc.createElement('div');
  who.className = 'who';
  who.textContent = (p.envelope && p.envelope.name) || '';
  if (who.textContent) el.appendChild(who);
  // **谁在等、前面还排着几条**：队首之后还在等的那几张如实列出来（只有队首能答）。
  const waiting = p.waiting || [];
  if (waiting.length) {
    const wq = doc.createElement('div');
    wq.className = 'queue';
    wq.textContent =
      '后面还排着 ' + waiting.length + ' 张（先答上面这张）：' +
      waiting.map((w) => ((w.envelope && w.envelope.name) || '') + ' 在等 ' + (w.title || '')).join('；');
    el.appendChild(wq);
  }
  const q = doc.createElement('div');
  q.className = 'q';
  q.textContent = msg.title || '';
  el.appendChild(q);
  if (msg.body) {
    const bd = doc.createElement('div');
    bd.className = 'hint';
    bd.textContent = msg.body;
    el.appendChild(bd);
  }
  if (msg.detail) {
    const dt = doc.createElement('div');
    dt.className = 'advice';
    dt.textContent = msg.detail;
    el.appendChild(dt);
  }
  const inp = doc.createElement('input');
  inp.className = 'decision-input';
  inp.placeholder = '附言（可空；要你回话 / 先说一句的那一关必填）…';
  inp.disabled = busy;
  el.appendChild(inp);
  const bs = doc.createElement('div');
  bs.className = 'btns';
  for (const o of (p.options || [])) {
    const bt = doc.createElement('button');
    bt.className = 'btn btn-primary';
    bt.textContent = o.label;
    bt.disabled = busy;
    bt.onclick = () => answerCard(p.id, o.id, inp.value.trim());
    bs.appendChild(bt);
  }
  el.appendChild(bs);
  return el;
}

/* 回答一张裁决卡：**选项 id 是契约**（前端只把它发回去），附言是用户想说的话。
   回答只有这一条命令（卡片 id + 选项 id），校验属于当时那张卡由后端做。 */
async function answerCard(card, option, note) {
  const s = activeSession();
  if (!s || s.readonly || s.sending) return;
  if (note) s.lines.push({ cls: 'user', who: '用户', text: note, pending: true });
  s.sending = true;
  renderStream();
  renderGate(s);
  try {
    await api('POST', actionUrl('answer_card'), { session_id: s.sid, card: card, option: option, note: note });
    renderAll();
  } catch (err) {
    s.lines = s.lines.filter((x) => !x.pending);
    s.forceFull = true;
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    renderAll();
  } finally {
    s.sending = false;
    needState = true;
    renderAll();
  }
}

/* 改需求：**用户自己点的动作**（不是核心推的门）。
   服务端清掉旧需求行、在末尾追加新需求（不回档、不动文件），返回完整重放，前端整体重建。 */
async function updateTaskFlow() {
  const s = activeSession();
  if (!s || isBusy(s) || !s.can_update_task) return;
  const v = takeInput();
  if (!v) {
    notice('改需求', '先在输入框写下新的本次需求，再点「改需求」。', 'info');
    return;
  }
  await updateTask(v);
}

async function updateTask(text) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  try {
    const r = await api('POST', actionUrl('update_task'), { session_id: s.sid, text });
    s.lines = []; s.pending = null; s.readonly = false; s.done = false;
    s.forceFull = true;
    for (const ev of (r.events || [])) absorb(s, ev);
    renderAll();
    ui.toast('ok', '已改需求');
  } catch (err) { notice('操作失败', err.message, 'err'); }
}

function gateCard(q, btns) {
  const el = doc.createElement('div');
  el.className = 'gate-card';
  const qq = doc.createElement('div'); qq.className = 'q'; qq.textContent = q; el.appendChild(qq);
  const bs = doc.createElement('div'); bs.className = 'btns';
  for (const [label, fn] of btns) {
    const b = doc.createElement('button'); b.className = 'btn'; b.textContent = label;
    b.onclick = () => fn().catch((err) => notice('操作失败', err.message, 'err'));
    bs.appendChild(b);
  }
  el.appendChild(bs);
  return el;
}

/* ---------- 动作 ---------- */
function takeInput() {
  const v = $('#input').value.trim();
  $('#input').value = '';
  atClose();
  autoGrow();
  return v;
}

/* 已应用到的批序号（**单调游标**）。
   事实只有一条来路（事件台）：批次按 seq 到达，游标只前进；服务端裁剪造成跳号时由
   `oldest` 触发一次历史重放重新对齐（见 pollLoop）。 */
let appliedSeq = 0;
/* 状态（侧栏/历史）可能被**别的客户端**改了：置位后由轮询统一拉一次。 */
let needState = false;
let lastStateAt = Date.now();
/** 事件流出现**补不齐的缺口**（服务端裁剪）后，按历史重放一次当前会话。
 * 为什么：spinner 与按钮都靠事件流，干等会让界面停在旧状态；重放比"攒着不显示"诚实。 */
async function resyncActive() {
  const s = activeSession();
  if (!s || s.readonly) return;
  try {
    const r = await api('GET', '/api/history/' + encodeURIComponent(s.sid));
    s.lines = []; s.live = [];
    // 重放的是**转录**（运行态是短暂事件，不在盘上）：这条会话的实时知识同样作废，按快照对账。
    s.running_known = false;
    // 重新合流：水位跟着刷新，之后的批次仍然只收水位之上的。
    if (typeof r.head === 'number') s.floor = r.head;
    for (const ev of (r.events || [])) absorb(s, ev);
    for (const ev of (r.live || [])) absorb(s, ev);
    renderStream(true);
  } catch { /* 拿不到就等下一次状态刷新 */ }
}

/** 收一批事件并应用。返回 'full' | 'live' | 'none'（渲染粒度）。 */
function applyBatch(seq, sid, events) {
  if (typeof seq === 'number') appliedSeq = Math.max(appliedSeq, seq);
  const s = state.sessions.get(sid);
  // **前端不认识的会话一律不动**（系统会话如核心推荐的 `#suggest` 也在内）：会话表只由
  // /api/state 给，前端只渲染它知道的会话。为事件里冒出来的 sid 就地建会话 = 前端自己造会话
  // 状态，非会话（系统会话）会因此长出多余标签页，而真会话的记录又会被建重一遍。
  // 别的客户端建的**真**会话由下一次状态刷新带进来（refreshState 建 + 合流补齐）。
  if (!s) {
    needState = true;
    return 'none';
  }
  // 水位以下的批次已经在**合流**里给过（/api/history 一次给全）：丢掉，不是"没收到"。
  if (typeof seq === 'number' && typeof s.floor === 'number' && seq <= s.floor) return 'none';
  // 合流还没落定：先攒着，等水位出来再按水位筛（见 hydrateHistory）。
  if (s.hydrating) {
    if (!s.buffer) s.buffer = [];
    s.buffer.push({ seq, events });
    return 'none';
  }
  // 运行态**不从增量猜**：核心在起止各推一条 `working`（同一个批次里就在 events 里），
  // 事件按序吸收，运行态因此自己就对了——猜"有增量=在跑"只会与事件打架。
  return absorbEvents(s, events);
}
/* 本地名 → 动作 id 的映射：动作目录与校验都在核心（systools/tools.yaml），这里只做转写。 */
const ACT_IDS = { say: 'send_message', task: 'set_task' };
/* 动作只有一个入口 /api/actions/{id}：声明、授权、执行与审计都在核心那一处。 */
function actionUrl(id) { return '/api/actions/' + id; }
async function act(action, text) {
  const s = activeSession();
  if (!s) return;
  if (isBusy(s)) return;
  if (s.readonly) { notice('历史回放为只读', '点「启用会话」后才能发送消息。', 'info'); return; }
  // 本地事实：我的命令在途（不是服务端运行态）——按钮立刻切「停止」，避免重复提交。
  s.sending = true;
  // 乐观回显：自己的发言立刻可见；服务端权威行到达时自动替换（见 absorb）。
  if (text && (action === 'say' || action === 'task' || action === 'answer')) {
    s.lines.push({ cls: 'user', who: '用户', text, pending: true });
  }
  renderStream();
  try {
    // 命令回包只有头部序号：事实（含自己那条发言的权威行）由事件流补进来。
    await api('POST', actionUrl(ACT_IDS[action] || action), { session_id: s.sid, text });
    s.awaiting = null;
    renderAll();
  } catch (err) {
    s.lines = s.lines.filter((x) => !x.pending);
    s.forceFull = true;
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    renderAll();
  } finally {
    s.sending = false;
    // 回包只给头部序号；事实由事件流补进来。顺手要一次快照**对账**：命令已结束但它的
    // 收尾事件可能还在路上，运行态由事件（或这轮快照）说了算，不由"我在不在途"说了算。
    needState = true;
    renderAll();
  }
}

/* 回档：留档（默认，可恢复）或删除（真的截断）；服务端回重放后的完整事件流，前端整体重建。 */
function rewindTo(id) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  app.choiceModal('回档到此处', '留档：保留内容、折叠成标记，可随时恢复，工作区文件（共享区与各 agent 沙箱）也一起记入档案；删除：真的删掉这一行及其后的全部，工作区文件一起回到该行（未提交内容进 lost 备份），删除点之后的留档一并作废，不可恢复。', [
    ['留档', 'btn', () => applyRewind(s, { id, mode: 'archive' })],
    ['删除', 'btn btn-danger', () => applyRewind(s, { id, mode: 'delete' })],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 恢复某次留档：删掉该标记及其后的内容（含留档期间的新工作）。 */
function restoreMark(mark) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  app.choiceModal('恢复这次留档', '恢复到留档之前：该标记及其后的全部内容（含留档期间的新工作）会被删除；工作区文件（共享区与各 agent 沙箱）一起回到留档点，未提交的改动进 lost 备份。', [
    ['恢复', 'btn btn-danger', () => applyRewind(s, { id: mark, mode: 'restore' })],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 回档请求的统一收尾：服务端回重放事件，前端整体重建。 */
async function applyRewind(s, body) {
  try {
    const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid }, body));
    if (typeof r.head === 'number') s.floor = r.head;
    // 行整体重建，折叠状态一并重来（避免旧键被新行复用）；与事件台上的 `rewound` 同一套逻辑。
    applySnapshot(s, (r.events || []).concat(r.live || []));
    s.workDirty = true;
    renderAll();
    ui.toast('ok', '已回档');
  } catch (err) {
    if (/被外部改动/.test(err.message) &&
        await confirmBox('覆盖工作区外部改动', err.message + '\n\n确认后会先采纳这些外部改动，再把工作区回档到目标提交；未跟踪文件不受影响。', '仍要回档')) {
      try {
        const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid, force: true }, body));
        if (typeof r.head === 'number') s.floor = r.head;
        applySnapshot(s, (r.events || []).concat(r.live || []));
        s.workDirty = true;
        renderAll();
        ui.toast('ok', '已确认覆盖并回档');
      } catch (e2) { notice('操作失败', e2.message, 'err'); }
      return;
    }
    if (/个留档/.test(err.message) &&
        await confirmBox('删除会作废留档', err.message + '\n\n确认后会连同共享区与各 agent 沙箱一起回到该行；这些留档作废后不可恢复。', '仍要删除')) {
      try {
        const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid, force: true }, body));
        if (typeof r.head === 'number') s.floor = r.head;
        applySnapshot(s, (r.events || []).concat(r.live || []));
        s.workDirty = true;
        renderAll();
        ui.toast('ok', '已确认删除');
      } catch (e2) { notice('操作失败', e2.message, 'err'); }
      return;
    }
    notice('操作失败', err.message, 'err');
  }
}

/* 撤回某 agent 的同意（转录追加撤回行，协作才有意义）；值 = agent 实例名。 */
function withdrawAgree(agent) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  app.choiceModal('撤回同意', '撤回「' + agent + '」的同意？继续时会按剩余转录重新判定。', [
    ['撤回', 'btn btn-danger', async () => {
      try {
        await api('POST', actionUrl('withdraw'), { session_id: s.sid, agent });
        renderAll();
        ui.toast('ok', '已撤回「' + agent + '」的同意');
      } catch (err) { notice('操作失败', err.message, 'err'); }
    }],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 启用：由用户点击授权核心恢复一条历史会话。后端负责重建 / 共享 guest / 解冻整棵树；
   前端只把返回的会话视图贴回本地并渲染。启用本身不跑模型。 */
async function activateFlow() {
  const s = activeSession();
  if (s && s.workspace_missing) { notice('工作区已不存在', '这条会话绑定的工作区已不可达：恢复目录或删除会话后再启用。', 'err'); return; }
  if (!s || !s.readonly || s.activating) return;
  s.activating = true;
  renderAll();
  try {
    const r = await api('POST', actionUrl('activate_session'), { session_id: s.sid });
    applyActivation(s, (r && r.session) || {});
    s.readonly = false;
    renderAll();
    ui.toast('ok', '会话已启用');
  } catch (err) {
    notice('启用失败', err.message, 'err');
  } finally {
    s.activating = false;
    renderAll();
  }
}

function applyActivation(s, v) {
  s.mode = v.mode || s.mode;
  s.run = v.run || s.run || 'active';
  s.running = !!v.running;
  s.running_known = true;
  s.done = !!v.done;
  s.can_update_task = !!v.can_update_task;
  s.pending = v.pending || null;
  s.continue_action = v.continue || null;
  s.tier = v.tier || s.tier;
  s.tier_ready = v.tier_ready;
  s.tier_missing = v.tier_missing || [];
  // 启用返回的 SessionView 已经带生效权限：和会话状态一起贴回，别等下一次 /api/state 轮询。
  s.permission_profile = v.permission_profile || s.permission_profile;
  s.permission_label = v.permission_label || s.permission_label;
  s.permission_narrowed = !!v.permission_narrowed;
  s.permission_parent = v.permission_parent || null;
  // 启用时后端已按转录回填 shell 快照：贴回来，不用等下一次 /api/state。
  if (v.shell) s.shell = v.shell;
}

/* 上下文「继续」：后端说确实有断点时才出现（单 agent 上一轮 / 协作泵）。
   效果仍是 control_session 的 continue；没有通用按钮。 */
async function continueContext() {
  const s = activeSession();
  if (!s || s.readonly || isBusy(s) || !s.continue_action) return;
  const hint = s.continue_action;
  s.sending = true;
  s.continue_action = null; // 先收起；以事件与快照为准，失败时状态刷新会再给
  renderAll();
  try {
    await api('POST', actionUrl('control_session'), {
      session_id: s.sid,
      action: 'continue',
      reason: hint.reason || hint.label || '',
    });
  } catch (err) {
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    renderAll();
  } finally {
    s.sending = false;
    needState = true; // 运行态以事件为准，快照只对账
    renderAll();
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

/* ---------- 发送 ---------- */
$('#btn-send').onclick = onSend;
$('#btn-continue').onclick = continueContext;
$('#btn-update-task').onclick = updateTaskFlow;
$('#input').addEventListener('keydown', (e) => {
  if (atKey(e)) return; // 菜单打开时 ↑/↓/Enter/Esc 归菜单
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); onSend(); }
});
$('#input').addEventListener('input', () => { autoGrow(); atOnInput(); });
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
  if (s.readonly) { notice('历史回放为只读', '点「启用会话」后才能发送消息。', 'info'); return; }
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
    notice('请按卡片作答', '这一关请点卡片上的选项；要补充的话写在卡片里的附言框。', 'info');
    return;
  }
  if (s.mode !== 'collab') { const v = takeInput(); if (v) act('say', v); return; } // 单 agent 形态可以自由发言
  // 协作无挂起时忽略发送（避免打断泵）。
}

/* ---------- 长轮询：增量事件 + 连接状态 ---------- */
/* 传输（取批次、游标、断线重试）在 app/core/events.js；这里只管裁剪对齐与批次的应用。
 * 取事件失败才算断线（setConn）；应用 / 渲染出错走 eventError，连接照常是好的。 */
const eventStream = core.createEventStream({
  onConn: setConn,
  onTrim: trimAlign,
  onLines: applyLines,
  onError: eventError,
});
function pollLoop() { return eventStream.run(); }

/** 事件台裁剪过：since 之后有一段永久丢了。丢掉滞留，跳到还留着的起点，并按历史重放当前会话。 */
async function trimAlign(oldest) {
  if (!(oldest > 0 && oldest > appliedSeq + 1)) return;
  appliedSeq = oldest - 1;
  // 运行态的实时知识作废（收尾那条可能就在丢掉的那段里），退回用快照对账。
  for (const cur of state.sessions.values()) cur.running_known = false;
  needState = true;
  await resyncActive();
}

async function applyLines(lines) {
  let mode = 'none';
  for (const item of lines) {
    try {
      const m = applyBatch(item.seq, item.sid, item.events);
      if (m === 'full') mode = 'full';
      else if (m === 'live' && mode !== 'full') mode = 'live';
      // 运行态只在 applyBatch 里由事件改：这里碰它，迟到的流式增量会把「停止」误翻回「发送」。
    } catch { /* 单行损坏不拖垮轮询 */ }
  }
  // 别的客户端（演示脚本、另一个标签页）建 / 删 / 改会话时侧栏自己跟上：未知会话立刻置 needState，其余靠 3 秒兜底。
  if (needState || Date.now() - lastStateAt > 3000) {
    needState = false;
    lastStateAt = Date.now();
    await refreshState();
  }
  // 纯流式增量只 append 新节点（折叠、<pre> 滚动、外层滚动都不被打断）。
  if (mode === 'full') renderAll();
  else if (mode === 'live') scheduleLiveTick();
}
function setConn(ok, err) {
  $('#conn-state .dot').className = 'dot ' + (ok ? 'dot-ok' : 'dot-bad');
  $('#conn-text').textContent = ok ? '已连接' : '重连中…';
  if (!ok) console.warn('事件流连接失败：', err || '');
}

/// 事件应用/渲染出错：**不是断线**（连接好着，是界面自己没处理对）。
/// 同一个错只弹一次（否则每 3 秒一次会刷屏），但绝不静默吞掉——先落控制台，再如实告诉用户。
let lastEventError = '';
function eventError(err) {
  console.error('事件处理出错：', err);
  const msg = String((err && err.message) || err);
  if (msg === lastEventError) return;
  lastEventError = msg;
  try { notice('界面处理事件出错', msg, 'err'); } catch { /* 弹窗本身出问题就只剩控制台 */ }
}

/* ---------- 设置⑥：模块与服务（列表 + 运行 + 看回执） ---------- */
/* 事实只有两份：模块清单 + 服务运行态（/api/state）与动作目录（/api/actions）——不在这里另写一份。
   只列 modules/ 下的模块；每个模块下面挂它的服务（带开关）与功能（每个功能一个「使用」子卡片）。
   系统工具各回各的入口，不倒进这一块（模块的全局开关 set_module_enabled 挂在模块头上）。 */

/* ---------- 抽屉遮罩 / 顶层接线 ---------- */
$('#drawer-mask').onclick = closeDrawer;
$('#btn-new-work').onclick = openWizard;
$('#btn-settings').onclick = () => app.openSettingsHub('generate');
$('#fence-settings').onclick = () => app.openSettingsHub('sandbox');
$('#btn-upload').onclick = () => app.pickUploadFile();
setupSidebar();

/* ---------- 启动 ---------- */
refreshState()
  .then(pollLoop)
  .catch((err) => notice('初始化失败', err.message, 'err'));
