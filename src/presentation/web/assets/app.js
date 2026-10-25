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

const $ = (s) => document.querySelector(s);
const state = {
  modules: [], providers: [], models: [], core: null, rejected: [], agents: [],
  history: [],           // 会话历史（名字/mode/时间）
  settings: { streaming: true, show_reasoning: true, llm_timeout_secs: 300, discuss_remind_cap: 3, compact_at_percent: 70 }, // 基本设置
  fence: null,           // /api/state 的围栏事实：只有降级 / 要求未满足时进横幅
  toolchains: null,      // /api/state 的基础工具链事实：项目内 / 宿主回退 / 缺失
  sessions: new Map(),   // sid -> { sid, mode, title, lines, pending, sending, running, working, done, awaiting, readonly }
  activeSid: null,
  treeOpen: new Set(),   // 会话树展开的父会话（纯 UI 状态，不落盘）
  autoOpenedFor: null,   // 上次自动展开过的活跃会话：只在切换时展开祖先，不抢用户手的折叠
  rail: false,           // 侧栏是否收起为图标栏（偏好记 localStorage）
  nodeBySid: new Map(),  // 子会话 sid -> {node, assignee}：节点卡与标签共用同一份标题
};

/* ---------- API ---------- */
async function api(method, url, body) {
  const res = await fetch(url, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = new Error(data.error || ('HTTP ' + res.status));
    err.status = res.status;
    err.data = data;
    throw err;
  }
  return data;
}

/* ---------- 状态与侧栏 ---------- */
async function refreshState() {
  const s = await api('GET', '/api/state');
  state.modules = s.modules || [];
  state.providers = s.providers || [];
  state.models = s.models || [];
  state.core = s.core || null;
  state.rejected = s.rejected || [];
  state.agents = s.agents || [];
  state.history = s.history || [];
  // 服务端的**权威会话视图**：有没有"本次需求" / 有没有等裁决。
  // 事件流给增量（推），这里是刷新后照样成立的快照（拉）——两面同一个事实。
  state.views = new Map((s.sessions || []).map((v) => [v.sid, v]));
  // 运行态的**对账副本**：事件是唯一真相（见 isBusy）；这份快照只在"本页对这条会话还没有实时
  // 知识"时用——刚刷新页面、事件台有缺口、别人建的会话。
  state.running = new Set(
    [].concat(
      (s.sessions || []).filter((v) => v.running).map((v) => v.sid),
      (state.history || []).filter((h) => h.running).map((h) => h.name),
    ),
  );
  // **会话表由后端给**：这里只为"后端说在跑"的会话建标签页。前端绝不为事件流里冒出来的 sid
  // 造会话状态——那是前端自己造会话（系统会话就是这么长出多余标签页的）。
  let opened = false;
  for (const v of state.views.values()) {
    if (state.sessions.has(v.sid) || !state.running.has(v.sid)) continue;
    const h = (state.history || []).find((x) => x.name === v.sid);
    const s = {
      sid: v.sid, mode: v.mode || 'single', title: (h && h.name) || v.sid, run: v.run || 'active',
      parent: (h && h.parent) || null,
      lines: [], live: [], pending: v.pending || null, sending: false,
      running: false, working: null, running_known: false,
      can_update_task: !!v.can_update_task,
      continue_action: v.continue || null,
      tier: v.tier, tier_ready: v.tier_ready, tier_missing: v.tier_missing || [],
      activating: false,
      done: !!(h && h.done), awaiting: null, fold: {}, scroll: {},
      readonly: false, hydrated: false, buffer: [],
    };
    state.sessions.set(v.sid, s);
    // 盘上转录是这条会话的**前缀**：补上它，之后的实时事件按 seq 接上（同 id 的行不重复）。
    hydrateHistory(s);
    opened = true;
  }
  // 已打开的标签页跟着快照对齐（增量事件到达时会覆盖成同一份）。
  for (const cur of state.sessions.values()) {
    const v = state.views.get(cur.sid);
    if (!v) continue;
    cur.can_update_task = !!v.can_update_task;
    cur.run = v.run || 'active';
    cur.pending = v.pending || null;
    // 上下文继续与档位提示都以后端视图为准；只读视图不显示继续（只有启用）。
    if (!cur.readonly) cur.continue_action = v.continue || null;
    cur.tier = v.tier;
    cur.tier_ready = v.tier_ready;
    cur.tier_missing = v.tier_missing || [];
    // 运行态**只对账、不覆盖**：这条会话已经有实时知识（事件就是真相）时，快照比它滞后——
    // 一次迟到的轮询不能把已经收尾的回合标回"在跑"。没有实时知识时按快照补齐，并清掉本地遗留
    // （快照说没跑就是没跑）；谁在跑未知时照旧显示"正在工作…"。
    if (!cur.running_known) {
      cur.running = snapshotRunning(cur.sid);
      if (!cur.running) cur.working = null;
    }
  }
  state.settings = s.settings || { streaming: true, show_reasoning: true, llm_timeout_secs: 300, discuss_remind_cap: 3, compact_at_percent: 70 };
  state.fence = s.fence || null;
  state.toolchains = s.toolchains || null;
  renderFenceBanner();
  renderToolchainBanner();
  renderSidebar();
  renderHistory();
  if (opened) renderTabs();
  const cur = activeSession();
  if (cur) { syncTyping(cur); syncSendButton(cur); syncComposer(cur); }
}

/// 思维链块：永远默认折叠，点击（原生 details）才展开。
/// Markdown 容器：有渲染器就渲染，没有就退回纯文本（安全）。
function mdNode(text) {
  const d = document.createElement('div');
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
  const body = document.createElement('div');
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
  const d = document.createElement('details'); d.className = 'tool-raw';
  foldBind(sess, key, d);
  const s = document.createElement('summary'); s.textContent = label;
  const pre = document.createElement('pre'); pre.className = 'tool-pre';
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
  const el = document.createElement('div');
  el.className = 'line tool-card ' + (info.ok ? 'ok' : 'bad');
  el.title = info.speaker ? info.speaker + ' 调用工具' : '工具调用';
  // 左上角先标出**是谁**在调工具：与其它行同一条规则（每个 kind 都带身份）。
  const speaker = who || info.speaker || '';
  if (speaker) {
    const w = document.createElement('span'); w.className = 'who'; w.textContent = speaker; el.appendChild(w);
  }
  const head = document.createElement('div'); head.className = 'tool-head';
  const name = document.createElement('span'); name.className = 'tool-name';
  name.textContent = (info.module ? info.module + '.' : '') + (info.name || '工具');
  const mark = document.createElement('span'); mark.className = 'tool-mark'; mark.textContent = info.ok ? '✓' : '✗';
  head.appendChild(name); head.appendChild(mark);
  el.appendChild(head);
  el.appendChild(toolDetail('参数', info.args, sess, key + ':args'));
  el.appendChild(toolDetail('结果', info.output, sess, key + ':output'));
  if (typeof info.raw === 'string' && info.raw.trim()) el.appendChild(toolDetail('原文', info.raw, sess, key + ':raw', true));
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

/// 正则特殊字符转义：根路径里可能有 . + ( 之类。
function escapeRe(s) {
  const special = '\\^$.*+?()[]{}|';
  return String(s).split('').map((c) => (special.indexOf(c) >= 0 ? '\\' + c : c)).join('');
}

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
  const alt = list.map((it) => escapeRe(it.root)).join('|');
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
  const d = document.createElement('details');
  d.className = 'cot';
  foldBind(sess, key, d);
  const s = document.createElement('summary'); s.textContent = '思维链';
  const b = document.createElement('div'); b.className = 'cot-body'; b.textContent = text;
  d.appendChild(s); d.appendChild(b);
  return d;
}

function checkbox(labelText, checked) {
  const wrap = document.createElement('label'); wrap.className = 'chk';
  const box = document.createElement('input'); box.type = 'checkbox'; box.checked = !!checked;
  const span = document.createElement('span'); span.textContent = labelText;
  wrap.appendChild(box); wrap.appendChild(span);
  return { wrap, box };
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
      const row = document.createElement('div');
      row.className = 'history-item';
      row.title = '点击打开会话';
      row.onclick = () => openHistory(h.name);
      const kids = by.get(h.name) || [];
      const open = state.treeOpen.has(h.name);
      if (kids.length) {
        const t = document.createElement('button');
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
        const sp = document.createElement('span'); sp.className = 'history-toggle'; row.appendChild(sp);
      }
      const name = document.createElement('span');
      name.className = 'hname'; name.textContent = h.name; name.title = h.name;
      const mode = document.createElement('span'); mode.className = 'hmode';
      // 运行态是**持久事实**（停止 / 关闭），与"这一刻在不在跑"分开：侧栏据此标出来。
      const stateTag = h.run === 'stopped' ? '·已停止'
        : h.run === 'closed' ? '·已关闭'
          : (h.done ? '' : '·进行中 ');
      mode.textContent = stateTag + h.mode +
        (h.tier === 'vm' && h.tier_ready === false ? '·虚拟机档不可用' : '');
      const pencil = ui.iconButton({ icon: 'pencil', label: '会话操作', tip: '', cls: 'hpen' });
      pencil.onclick = (e) => { e.stopPropagation(); ui.menu(pencil, historyMenu(h)); };
      row.appendChild(name); row.appendChild(mode); row.appendChild(pencil);
      into.appendChild(row);
      if (kids.length && open) {
        const wrap = document.createElement('div'); wrap.className = 'history-kids';
        branch(h.name, wrap);
        into.appendChild(wrap);
      }
    }
  };
  branch('', box);
}

/// 快照（/api/state）说这条会话、或它的任一子会话在跑吗。
/// **对账副本的唯一读法**：只在 isBusy 的第三档（本页没有实时知识）用。
function snapshotRunning(sid) {
  const run = state.running;
  if (!run || !sid) return false;
  if (run.has(sid)) return true;
  const pre = sid + '--';
  for (const k of run) if (k.indexOf(pre) === 0) return true;
  return false;
}

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
      done: true, awaiting: null, readonly: true, fold: {}, scroll: {},
    };
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
  const d = document.createElement('div'); d.className = cls;
  if (text != null) d.textContent = text;
  return d;
}
function cfgHint(text, kind) {
  return cfgDiv('cfg-hint' + (kind ? ' ' + kind : ''), text);
}
/// 执行档位二选一：原生 radio（同名即互斥）。disabled = 本机承载不了（例如虚拟机档的前置条件不具备）。
function cfgRadio(labelText, checked, group, disabled) {
  const wrap = document.createElement('label'); wrap.className = 'chk';
  const box = document.createElement('input');
  box.type = 'radio'; box.name = group; box.checked = !!checked; box.disabled = !!disabled;
  const span = document.createElement('span'); span.textContent = labelText;
  wrap.appendChild(box); wrap.appendChild(span);
  return { wrap, box };
}

/// 从会话列表的「编辑」进入：面板自己拉配置、自己重绘（保存 / 重新扫描都走这里）。
function openConfig(sid) {
  openModal('配置：' + sid, (c) => {
    const box = document.createElement('div'); box.className = 'cfg';
    c.body.appendChild(box);
    loadConfig(sid, c, box);
  }, true);
}

async function loadConfig(sid, c, box) {
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
  buildConfigForm(sid, cfg, c, box);
}

/* ---------- 依赖补齐：把"缺什么"变成"点一下装上" ----------
 * 判据全在后端：`/api/setup/status` 的 `pending` 决定装什么（界面不自己挑条目——挑错了会把已装的重下一遍）。
 * 这里只做两步确认（先看清单、再下载）与进度；装完即生效，不需要重启。
 * 进度必须**看得见**：作业一起就进进度视图（当前步骤 + 滚动日志 + 取消）；关掉窗再打开也接着显示正在跑的那一次。 */
function openSetupModal(onDone) {
  openModal('补齐虚拟机依赖', (c) => {
    const box = document.createElement('div'); box.className = 'cfg';
    c.body.appendChild(box);
    const log = document.createElement('div'); log.className = 'cfg-log';
    c.body.appendChild(log);
    const finish = () => { c.close(); if (onDone) onDone(); };

    // 依赖状态 → 三种视图：已在跑（进度）/ 要装什么（清单）/ 都齐了。
    async function load() {
      box.innerHTML = '';
      log.textContent = '';
      box.appendChild(cfgHint('正在读本机的依赖状态…'));
      let st = null;
      try { st = await api('GET', '/api/setup/status'); }
      catch (e) {
        box.innerHTML = '';
        box.appendChild(cfgHint('读不到依赖状态：' + e.message, 'err'));
        return;
      }
      box.innerHTML = '';
      const job = st.job || {};
      // 关掉窗又打开：这一次还在跑就接着显示它，而不是再起一个（后端会拒绝第二个作业）。
      if (job.running) { watch(st); return; }
      const pending = Array.isArray(st.pending) ? st.pending : null;
      if (!pending) {
        box.appendChild(cfgHint('这个进程没有给出要补哪些条目（界面与核心版本不一致）：重启应用后再试。', 'err'));
        const close = btn('关闭', 'btn btn-block'); close.onclick = finish;
        box.appendChild(close);
        return;
      }
      if (!pending.length) {
        box.appendChild(cfgHint('依赖都已在场：不需要补齐。', 'ok'));
        const close = btn('关闭', 'btn btn-block'); close.onclick = finish;
        box.appendChild(close);
        return;
      }
      const deps = (st.deps || []).filter((d) => pending.indexOf(d.id) >= 0);
      box.appendChild(cfgDiv('cfg-sec-title', '要装 ' + deps.length + ' 条（只写项目内，不动系统）'));
      let total = 0;
      for (const d of deps) {
        total += d.size_mb || 0;
        const line = document.createElement('div');
        line.className = 'cfg-line';
        line.textContent = '· ' + d.id + (d.size_mb ? '（约 ' + d.size_mb + ' MB）' : '') + (d.note ? '：' + d.note : '');
        box.appendChild(line);
        if (d.url) { const u = document.createElement('div'); u.className = 'cfg-dir'; u.textContent = d.url; box.appendChild(u); }
      }
      box.appendChild(cfgHint('合计约 ' + total + ' MB：从网络下载后解包进项目内的 .vm/。'));
      const go = btn('开始下载安装', 'btn btn-primary btn-block');
      go.onclick = () => start(pending, go);
      box.appendChild(go);
      const later = btn('取消', 'btn btn-block'); later.onclick = c.close;
      box.appendChild(later);
    }

    async function start(ids, go) {
      go.disabled = true;
      c.setMsg('正在起补齐作业…');
      try { await api('POST', '/api/setup/ensure', { deps: ids, confirm: true }); }
      catch (e) { go.disabled = false; c.setMsg('起作业失败：' + e.message, true); return; }
      c.setMsg('作业已开始：正在下载并解包（进度见下面）');
      watch({ job: { seq: 0, running: true } });
    }

    // 进度视图：当前步骤 + 滚动日志 + 取消。轮询是长连接（服务端挂到有新行或 20s 到点）。
    function watch(st) {
      box.innerHTML = '';
      log.textContent = '';
      box.appendChild(cfgDiv('cfg-sec-title', '正在补齐（只写项目内）'));
      const step = cfgHint('等待第一条进度…');
      box.appendChild(step);
      const cancel = btn('取消作业', 'btn btn-block');
      cancel.onclick = async () => {
        try { await api('DELETE', '/api/setup/job'); c.setMsg('已请求取消，等作业收尾…'); }
        catch (e) { c.setMsg('取消失败：' + e.message, true); }
      };
      box.appendChild(cancel);
      let since = (st.job && st.job.seq) || 0;
      (async function poll() {
        for (;;) {
          let now = null;
          try { now = await api('GET', '/api/setup/status?since=' + since + '&wait=1'); }
          catch (e) { c.setMsg('读进度失败：' + e.message, true); return; }
          const job = now.job || {};
          if ((job.seq || 0) > since) since = job.seq;
          const lines = job.lines || [];
          log.textContent = lines.join('\n');
          if (lines.length) step.textContent = '现在：' + lines[lines.length - 1];
          if (!job.running) {
            if (job.error) {
              c.setMsg('补齐失败：' + job.error, true);
              step.textContent = '失败：' + job.error;
              return;
            }
            done(now);
            return;
          }
          c.setMsg('正在补齐…（可以先关掉这个窗，作业会继续跑；再打开还能接着看）');
        }
      })();
    }

    function done(st) {
      const left = Array.isArray(st.pending) ? st.pending : [];
      c.setMsg(left.length ? '作业结束了，但还有没装上的：' + left.join('、') : '补齐完成：依赖都齐了');
      box.innerHTML = '';
      const note = document.createElement('div'); note.className = 'cfg-hint ok';
      // 生效时机由后端保证（补齐完即按新状态判定，下一次起 guest 就用新的共享档）：这里只说结论。
      note.textContent = '已经生效：再打开配置 / 向导就按新状态判定，下一次启动虚拟机就用上，不需要重启。';
      box.appendChild(note);
      const again = btn('关闭', 'btn btn-block'); again.onclick = finish;
      box.appendChild(again);
    }

    load();
  }, true);
}

/// 结构化诊断 → 中文（呈现层文案，不进提示词册）。
function diagnosisText(d) {
  if (!d || typeof d !== 'object') return String(d);
  if (d.Missing) {
    return '模块 ' + d.Missing.module + ' 需要的运行能力 ' + d.Missing.capability +
      ' 在本档位没有：把它放进依赖文件夹后点「重新扫描」，或确认基础工具链 / 宿主 PATH。';
  }
  if (d.Ambiguous) {
    return '运行能力 ' + d.Ambiguous.capability + ' 有多个版本（' + (d.Ambiguous.versions || []).join('、') +
      '）：在下面「定版」里选一个版本，工具才跑得起来。';
  }
  if (d.UnknownPin) {
    return '运行能力 ' + d.UnknownPin.capability + ' 定的版本 ' + d.UnknownPin.version +
      ' 不在本档位可用的包里：改一个版本，或把该版本的运行包放进依赖文件夹后点「重新扫描」。';
  }
  if (d.Conflict) {
    return '运行包 ' + d.Conflict.a + ' 与 ' + d.Conflict.b + ' 都要写进 ' + d.Conflict.path +
      '：装配会互相覆盖，换版本或去掉其中一个。';
  }
  if (d.NotForTarget) {
    return '模块 ' + d.NotForTarget.module + ' 需要的运行能力 ' + d.NotForTarget.capability +
      ' 在本档位用不了：包库里只有 ' + (d.NotForTarget.packages || []).join('、') +
      '（目标：' + d.NotForTarget.target + '）。换一个包，或改用虚拟机档。';
  }
  if (d.NoBin) {
    return '运行包 ' + d.NoBin.package + ' 没有可执行的 bin/：本机档放不进 PATH。检查包内容或换一个包。';
  }
  if (d.Path) {
    return '运行包 PATH 合成失败：' + d.Path.why;
  }
  if (d.MissingEntry) {
    return '模块 ' + d.MissingEntry.module + ' 的工具 ' + d.MissingEntry.tool +
      ' 入口不在场：' + d.MissingEntry.command + '。先按模块 README 或 --build 编一次。';
  }
  return JSON.stringify(d);
}

function buildConfigForm(sid, cfg, c, box) {
  const frozen = !!cfg.started;
  const rt = cfg.runtime || {};
  const dir = cfg.runtimes_dir || '';
  // 表单草稿：保存从这里读；「重新扫描」整块重建，未保存的改动随之丢弃。
  const draft = {
    agents: (cfg.agents || []).map((a) => ({
      name: a.name || '', modules: (a.modules || []).slice(), model: a.model || '',
    })),
    tier: cfg.tier === 'vm' ? 'vm' : 'host',
    guest: cfg.guest || '',
    pins: {},
    permissions: cfg.permissions || { allow_read: [], allow_write: [], deny: [], granularity: 'ask', ask: [] },
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
  draft.agents.forEach((a, i) => secA.appendChild(cfgAgentBlock(a, i, frozen)));
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
  const pick = guestPicker(cfg.guests, draft.guest, (v) => { draft.guest = v; });
  const syncTier = () => {
    const vm = draft.tier === 'vm';
    const show = vm && !tierLocked;
    pick.wrap.className = show ? 'wf-field' : 'wf-field hidden';
    pick.hint.className = 'cfg-hint' + (show ? '' : ' hidden');
    pick.sync();
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
        const line = document.createElement('div');
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
    const setupBtn = btn('补齐虚拟机依赖…', 'btn btn-block');
    setupBtn.onclick = () => {
      try { openSetupModal(() => openConfig(sid)); }
      catch (e) { notice('打开补齐弹层失败', e.message, 'err'); }
    };
    secT.appendChild(setupBtn);
  }
  secT.appendChild(pick.wrap); secT.appendChild(pick.hint);
  box.appendChild(secT);
  syncTier();

  // 运行能力：declared / missing / available / diagnoses / rejected / rejected_packages
  box.appendChild(cfgRuntimeBlock(rt, dir, draft.tier, () => {
    c.setMsg('正在重新扫描模块清单与依赖文件夹…');
    loadConfig(sid, c, box);
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

  // 会话权限：路径范围 / 模块写 / 决定粒度（网络是全局开关，不在这里）
  const secPerm = cfgDiv('cfg-sec');
  secPerm.appendChild(cfgDiv('cfg-sec-title', '会话权限'));
  secPerm.appendChild(cfgHint('网络由全局 sandbox.policy.network 控制；本会话不可单独改。'));
  const perm = draft.permissions || {};
  const lines = (v) => (Array.isArray(v) ? v.join('\n') : '');
  const parseLines = (text) => String(text || '')
    .split(/[\n,]/).map((s) => s.trim()).filter((s) => s.length > 0);
  const makeList = (label, value, hint) => {
    const t = areaInput('每行一条');
    t.value = lines(value);
    t.addEventListener('input', () => { if (hint) hint.textContent = ''; });
    const f = field(label, t);
    secPerm.appendChild(f);
    return t;
  };
  const allowRead = makeList('读白名单（相对工作区根；空 = 整棵可读）', perm.allow_read);
  const allowWrite = makeList('提交白名单（相对工作区根；空 = 整棵可提交）', perm.allow_write);
  const deny = makeList('黑名单（读写都禁；全局黑名单是硬底线，这里只能追加）', perm.deny);
  const askList = makeList('需要确认的工具名（granularity=ask 时生效；模块工具写 module.tool）', perm.ask);
  const gran = selectInput([
    { value: 'ask', label: 'ask：ask 表里的工具先问用户' },
    { value: 'full', label: 'full：任何工具都不确认' },
  ], perm.granularity || 'ask');
  secPerm.appendChild(field('决定粒度', gran));
  box.appendChild(secPerm);

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
      agents: draft.agents.map((a) => ({ name: a.name.trim(), modules: a.modules.slice(), model: a.model || '' })),
      tier: draft.tier,
      guest: draft.tier === 'vm' ? (draft.guest || null) : null,
      pins: pins,
      permissions: {
        allow_read: parseLines(allowRead.value),
        allow_write: parseLines(allowWrite.value),
        deny: parseLines(deny.value),
        ask: parseLines(askList.value),
        granularity: gran.value,
      },
    };
    save.disabled = true;
    c.setMsg('保存中…');
    try {
      await api('POST', actionUrl('edit_session'), Object.assign({ session_id: sid }, body));
      await refreshState();
      c.setMsg('已保存：下一次发言按新配置生效');
      loadConfig(sid, c, box); // 重读一遍：把落盘后的样子如实显示出来
    } catch (e) {
      save.disabled = false;
      c.setMsg('保存失败：' + e.message, true); // 后端 error 原文，不吞
    }
  };
  acts.appendChild(save); acts.appendChild(cancel);
  box.appendChild(acts);
}

/// 一个 agent 的编辑块：名字（冻结时只读）/ 模块多选（选项来自 /api/state 的 modules）/ 模型下拉。
function cfgAgentBlock(a, idx, frozen) {
  const row = cfgDiv('cfg-agent');
  row.appendChild(cfgDiv('cfg-agent-head', 'agent ' + (idx + 1) + '：' + (a.name || '（未命名）')));

  const nameIn = textInput('agent 名字（也会当它的沙箱目录名）');
  nameIn.value = a.name || '';
  nameIn.disabled = frozen;
  nameIn.addEventListener('input', () => { a.name = nameIn.value; });
  row.appendChild(field(frozen ? '名字（只读）' : '名字', nameIn));
  if (frozen) row.appendChild(cfgHint('会话已经开过：名单与形态冻结 —— 名字改不了（要换人请新建会话）；模块与模型仍可改。', 'warn'));

  const mods = cfgDiv('wf-mods');
  if (!state.modules.length) mods.appendChild(emptyHint('（modules/ 下没有模块）'));
  for (const m of state.modules) {
    const lab = document.createElement('label'); lab.className = 'wf-check';
    const cb = document.createElement('input'); cb.type = 'checkbox';
    cb.checked = a.modules.indexOf(m.id) >= 0;
    cb.addEventListener('change', () => {
      if (cb.checked) { if (a.modules.indexOf(m.id) < 0) a.modules.push(m.id); }
      else a.modules = a.modules.filter((x) => x !== m.id);
    });
    const id = document.createElement('span'); id.textContent = m.id;
    const brief = document.createElement('span'); brief.className = 'wf-brief'; brief.textContent = m.brief || '';
    lab.appendChild(cb); lab.appendChild(id); lab.appendChild(brief);
    mods.appendChild(lab);
  }
  row.appendChild(field('模块（这个 agent 的能力；勾 1 个 = 直连式，勾多个 = 组合式；同一模块可被多个 agent 共用）', mods));
  const cur = a.model || '';
  const opts = [{ value: '', label: '（核心默认）' }]
    .concat(state.models.map((m) => ({ value: m.id, label: m.name + '（' + m.id + '）' })));
  if (cur && !state.models.some((m) => m.id === cur)) opts.push({ value: cur, label: cur + '（登记处已无此模型）' });
  const sel = selectInput(opts, cur);
  sel.addEventListener('change', () => { a.model = sel.value; });
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
    for (const d of diags) sec.appendChild(cfgDiv('cfg-line cfg-bad', '· ' + diagnosisText(d)));
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
  let msg = '基础工具链未安装：' + missing.join('、') + '。补全：node setup-runtime.js --yes';
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
  try { return typeof matchMedia === 'function' && matchMedia('(max-width: 899px)').matches; }
  catch (e) { return false; }
}
function readRail() {
  try { return localStorage.getItem(RAIL_KEY) === 'rail'; } catch (e) { return false; }
}
function saveRail() {
  try { localStorage.setItem(RAIL_KEY, state.rail ? 'rail' : 'wide'); } catch (e) { /* 存不了就只留内存态 */ }
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

/// 侧栏的图标与底部设置按钮：HTML 只留骨架，图标从 ui.js 来（不再写字符画）。
function setupSidebar() {
  if (typeof ui === 'undefined') return;
  const foot = $('#btn-settings');
  if (foot && !(foot.children || []).length) {
    foot.appendChild(ui.icon('gear'));
    const label = document.createElement('span');
    label.className = 'sidebar-action-label';
    label.textContent = '设置';
    foot.appendChild(label);
  }
  const rail = $('#sidebar-rail');
  if (rail && !(rail.children || []).length) {
    const expand = () => { applyRail(false); saveRail(); };
    rail.appendChild(ui.iconButton({ icon: 'plus', label: '新建工作', tip: '新建工作', onClick: openWizard }));
    rail.appendChild(ui.iconButton({ icon: 'clock', label: '会话历史', tip: '会话历史', onClick: expand }));
    rail.appendChild(ui.iconButton({ icon: 'gear', label: '设置', tip: '设置', onClick: () => openSettingsHub('generate') }));
    rail.appendChild(ui.iconButton({ icon: 'chevron', label: '展开侧栏', tip: '展开侧栏', onClick: expand }));
  }
  const drawer = $('#btn-drawer');
  if (drawer) {
    drawer.innerHTML = '';
    drawer.appendChild(ui.icon('panel'));
    drawer.onclick = toggleSidebar;
  }
  applyRail(readRail());
  if (typeof window !== 'undefined' && window.addEventListener) {
    window.addEventListener('resize', () => applyRail(state.rail));
  }
}

/* ---------- 通用小构件（只用冒烟桩支持的 API） ---------- */
function btn(label, cls) {
  const b = document.createElement('button');
  b.type = 'button'; b.className = cls || 'btn'; b.textContent = label;
  return b;
}
/* 把按钮置忙（转圈 + 禁用 + 换文案），返回复位函数。为什么要有：一件要等几十秒的事，
 * 界面不给反应就等于告诉用户「卡了」；复位放在 finally 里，失败也一定恢复可点。 */
function busyButton(b, label) {
  const was = b.textContent;
  const wasDisabled = !!b.disabled;
  b.disabled = true;
  b.classList.add('busy');
  if (label) b.textContent = label;
  return () => {
    b.classList.remove('busy');
    b.textContent = was;
    b.disabled = wasDisabled;
  };
}
function textInput(placeholder, type) {
  const i = document.createElement('input');
  i.className = 'field-input'; i.placeholder = placeholder || ''; i.autocomplete = 'off';
  if (type) i.type = type;
  return i;
}
/** 数字输入 + 标签 + 边界（设置里的"秒"这类：不许留空、不许越界）。 */
function numberInput(labelText, value, min, max) {
  const input = textInput('', 'number');
  input.min = String(min); input.max = String(max);
  input.value = String(value === undefined || value === null ? '' : value);
  return { input, wrap: field(labelText, input) };
}
function areaInput(placeholder) {
  const t = document.createElement('textarea');
  t.className = 'field-input'; t.rows = 3; t.placeholder = placeholder || '';
  return t;
}
function selectInput(options, value) {
  const s = document.createElement('select');
  s.className = 'field-input';
  for (const o of options) {
    const op = document.createElement('option');
    op.value = o.value; op.textContent = o.label;
    s.appendChild(op);
  }
  if (value != null) s.value = value;
  return s;
}
function field(labelText, input) {
  const wrap = document.createElement('div'); wrap.className = 'wf-field';
  const lab = document.createElement('div'); lab.className = 'wf-label'; lab.textContent = labelText;
  wrap.appendChild(lab); wrap.appendChild(input);
  return wrap;
}
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

function emptyHint(text) {
  const e = document.createElement('div'); e.className = 'reg-empty'; e.textContent = text;
  return e;
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
  const overlay = document.createElement("div"); overlay.className = "notice-overlay";
  const panel = document.createElement("div"); panel.className = "notice-panel notice-" + (kind || "info");
  const head = document.createElement("div"); head.className = "notice-title"; head.textContent = title || "";
  const body = document.createElement("div"); body.className = "notice-text"; body.textContent = text || "";
  const row = document.createElement("div"); row.className = "notice-actions";
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
function confirmBox(title, text, confirmLabel) {
  return new Promise((resolve) => {
    const root = $("#notice-root");
    root.innerHTML = "";
    root.className = "notice-root open";
    const overlay = document.createElement("div"); overlay.className = "notice-overlay";
    const panel = document.createElement("div"); panel.className = "notice-panel notice-warn";
    const head = document.createElement("div"); head.className = "notice-title"; head.textContent = title || "";
    const body = document.createElement("div"); body.className = "notice-text"; body.textContent = text || "";
    const row = document.createElement("div"); row.className = "notice-actions";
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

/* ---------- 主区弹层（清空一律 innerHTML=''） ---------- */
let modalUnbind = null;
function closeModal() {
  if (modalUnbind) { modalUnbind(); modalUnbind = null; }
  const root = $('#modal-root');
  root.innerHTML = '';
  root.className = 'modal-root';
}
function openModal(title, build, wide) {
  closeModal();
  const root = $('#modal-root');
  root.className = 'modal-root open';
  const overlay = document.createElement('div'); overlay.className = 'modal-overlay';
  const panel = document.createElement('div'); panel.className = 'modal-panel' + (wide ? ' wide' : '');
  const head = document.createElement('div'); head.className = 'modal-head';
  const h = document.createElement('div'); h.className = 'modal-title'; h.textContent = title;
  const close = btn('✕', 'modal-close');
  close.onclick = closeModal;
  head.appendChild(h); head.appendChild(close);
  const body = document.createElement('div'); body.className = 'modal-body';
  const msg = document.createElement('div'); msg.className = 'modal-msg';
  panel.appendChild(head); panel.appendChild(body); panel.appendChild(msg);
  overlay.appendChild(panel);
  root.appendChild(overlay);
  modalUnbind = ui.bindDismiss(overlay, panel, closeModal);
  const ctx = {
    body,
    setMsg: (t, isErr) => { msg.textContent = t || ''; msg.className = isErr ? 'modal-msg err' : 'modal-msg'; },
    close: closeModal,
  };
  build(ctx);
  return ctx;
}

/* ---------- 设置①：供应商登记（录入 / 修改 / 删除） ---------- */
function providersPage(c) {
    const listWrap = document.createElement('div'); listWrap.className = 'reg-list';
    const idIn = textInput('id');
    const urlIn = textInput('base_url');
    const keyIn = textInput('api_key（保存后不回显）', 'password');
    const save = btn('登记 / 更新', 'btn btn-primary btn-block');

    function rebuild() {
      listWrap.innerHTML = '';
      if (!state.providers.length) { listWrap.appendChild(emptyHint('（暂无供应商）')); return; }
      for (const p of state.providers) {
        const row = document.createElement('div'); row.className = 'reg-item';
        const main = document.createElement('div'); main.className = 'reg-main';
        const id = document.createElement('div'); id.className = 'reg-id'; id.textContent = p.id;
        const sub = document.createElement('div'); sub.className = 'reg-sub'; sub.textContent = p.base_url;
        main.appendChild(id); main.appendChild(sub);
        const acts = document.createElement('div'); acts.className = 'reg-acts';
        const edit = btn('编辑', 'link-btn');
        edit.onclick = () => {
          idIn.value = p.id; urlIn.value = p.base_url; keyIn.value = '';
          c.setMsg('编辑 ' + p.id + '：api_key 需重新填写');
        };
        const del = btn('删除', 'link-btn danger');
        del.onclick = async () => {
          try {
            await api('POST', actionUrl('remove_provider'), { id: p.id });
            await refreshState(); rebuild();
            c.setMsg('已删除 ' + p.id);
          } catch (e) { c.setMsg(e.message, true); }
        };
        acts.appendChild(edit); acts.appendChild(del);
        row.appendChild(main); row.appendChild(acts);
        listWrap.appendChild(row);
      }
    }

    save.onclick = async () => {
      const id = idIn.value.trim(), url = urlIn.value.trim(), key = keyIn.value;
      if (!id || !url || !key) { c.setMsg('id / base_url / api_key 均不能为空', true); return; }
      try {
        await api('POST', actionUrl('upsert_provider'), { id, base_url: url, api_key: key });
        keyIn.value = '';
        await refreshState(); rebuild();
        c.setMsg('已保存：' + id);
      } catch (e) { c.setMsg(e.message, true); }
    };

    rebuild();
    c.body.appendChild(listWrap);
    const form = document.createElement('div'); form.className = 'form-col';
    form.appendChild(field('id', idIn));
    form.appendChild(field('base_url', urlIn));
    form.appendChild(field('api_key', keyIn));
    form.appendChild(save);
    c.body.appendChild(form);
}

function openProvidersModal() {
  openModal('供应商登记', providersPage);
}

/* ---------- 设置②：模型登记（录入 / 修改 / 删除 + 供应商模型发现） ---------- */
function modelsPage(c) {
  const changed = () => { if (typeof c.afterChange === 'function') c.afterChange(); };
    const listWrap = document.createElement('div'); listWrap.className = 'reg-list';
    const discWrap = document.createElement('div'); discWrap.className = 'reg-list';
    const idIn = textInput('id');
    const nameIn = textInput('展示名');
    const apiIn = textInput('api_model（真正发给供应商的串）');
    const providerSel = selectInput(state.providers.map((p) => ({ value: p.id, label: p.id })), null);
    const noteIn = textInput('note（能力说明）');
    // 上下文窗口：自动压缩按它 × 设置里的百分比触发；填 0/留空 = 保留现值（新建缺省 32k）。
    const ctxIn = numberInput('上下文窗口（tokens）', 0, 0, 2000000);
    const save = btn('登记 / 更新', 'btn btn-primary btn-block');

    function rebuild() {
      listWrap.innerHTML = '';
      if (!state.models.length) { listWrap.appendChild(emptyHint('（暂无模型）')); return; }
      for (const m of state.models) {
        const row = document.createElement('div'); row.className = 'reg-item';
        const main = document.createElement('div'); main.className = 'reg-main';
        const id = document.createElement('div'); id.className = 'reg-id'; id.textContent = m.id;
        if (m.is_core) {
          const badge = document.createElement('span'); badge.className = 'reg-core'; badge.textContent = '核心';
          id.appendChild(badge);
        }
        const sub = document.createElement('div'); sub.className = 'reg-sub';
        sub.textContent = m.name + ' · ' + m.api_model + ' · 供应商 ' + m.provider +
          ' · 工具调用 ' + (m.tools === 'native' ? '原生' : '手写信封') +
          ' · 窗口 ' + (m.context || 0) + ' tokens';
        const note = document.createElement('div'); note.className = 'reg-note'; note.textContent = m.note || '';
        main.appendChild(id); main.appendChild(sub); main.appendChild(note);
        const acts = document.createElement('div'); acts.className = 'reg-acts';
        const edit = btn('编辑', 'link-btn');
        edit.onclick = () => {
          idIn.value = m.id; nameIn.value = m.name; apiIn.value = m.api_model;
          providerSel.value = m.provider; noteIn.value = m.note || '';
          ctxIn.input.value = m.context || 0;
          c.setMsg('编辑 ' + m.id);
        };
        const del = btn('删除', 'link-btn danger');
        del.onclick = async () => {
          try {
            await api('POST', actionUrl('remove_model'), { id: m.id });
            await refreshState(); rebuild(); changed();
            c.setMsg('已删除 ' + m.id);
          } catch (e) { c.setMsg(e.message, true); }
        };
        // 实测这条通道支不支持原生工具调用（要真实网络）：结论由后端如实回报，只写确定的结论。
        const probe = btn('测工具调用', 'link-btn');
        probe.onclick = async () => {
          probe.disabled = true;
          c.setMsg('正在实测 ' + m.id + ' 的工具调用支持（要发两条最小请求）…');
          try {
            const r = await api('POST', actionUrl('probe_model_tools'), { id: m.id });
            const what = r.outcome === 'supported' ? '支持原生工具调用'
              : r.outcome === 'unsupported' ? '**不支持**原生工具调用（供应商拒了带 tools 的请求）'
              : '无法判定（供应商接受了 tools，但这次没有发起调用）';
            await refreshState(); rebuild(); changed();
            c.setMsg(what + '｜' + (r.detail || '') + '｜登记处现在是「' +
              (r.mode === 'native' ? '原生' : '手写信封') + '」，下一次生成起生效');
          } catch (e) { c.setMsg(e.message, true); }
        };
        // 实测「工具调用历史怎么发回去」的几种写法（要真实网络；只报事实、不改登记处）。
        const probeReplay = btn('测回放形状', 'link-btn');
        probeReplay.onclick = async () => {
          probeReplay.disabled = true;
          c.setMsg('正在实测 ' + m.id + ' 的回放形状（几种写法各发一次）…');
          try {
            const r = await api('POST', actionUrl('probe_replay_shape'), { id: m.id });
            const lines = (r.shapes || []).map((s) => {
              const verdict = !s.accepted ? '被拒' : (s.understood ? '收+读懂' : '收未懂');
              return verdict + '  ' + s.name + '  ' + (s.detail || '');
            });
            c.setMsg('回放形状（' + m.id + '）：\n' + lines.join('\n'));
          } catch (e) { c.setMsg(e.message, true); }
        };
        acts.appendChild(probeReplay); acts.appendChild(probe); acts.appendChild(edit); acts.appendChild(del);
        row.appendChild(main); row.appendChild(acts);
        listWrap.appendChild(row);
      }
    }

    function rebuildDiscover() {
      discWrap.innerHTML = '';
      if (!state.providers.length) { discWrap.appendChild(emptyHint('（暂无供应商，先登记供应商）')); return; }
      for (const p of state.providers) {
        const row = document.createElement('div'); row.className = 'disc-row';
        const label = document.createElement('span'); label.className = 'disc-id'; label.textContent = p.id;
        const go = btn('获取模型列表', 'link-btn');
        const chips = document.createElement('div'); chips.className = 'chips';
        go.onclick = async () => {
          chips.innerHTML = '';
          c.setMsg('正在拉取 ' + p.id + ' 的模型列表…');
          try {
            const r = await api('POST', actionUrl('discover_models'), { id: p.id });
            const list = r.models || [];
            if (!list.length) { chips.appendChild(emptyHint('（未返回模型）')); }
            for (const name of list) {
              const chip = btn(name, 'chip');
              chip.onclick = () => {
                apiIn.value = name; providerSel.value = p.id;
                c.setMsg('已填入 api_model：' + name + '（供应商 ' + p.id + '）');
              };
              chips.appendChild(chip);
            }
            c.setMsg('获取到 ' + list.length + ' 个模型，点选填入 api_model');
          } catch (e) { c.setMsg(e.message, true); }
        };
        row.appendChild(label); row.appendChild(go); row.appendChild(chips);
        discWrap.appendChild(row);
      }
    }

    save.onclick = async () => {
      const body = {
        id: idIn.value.trim(), name: nameIn.value.trim(), api_model: apiIn.value.trim(),
        provider: providerSel.value, note: noteIn.value.trim(),
        context: Number(ctxIn.input.value) || 0,
      };
      if (!body.id || !body.name || !body.api_model || !body.provider) {
        c.setMsg('id / 名字 / api_model / 供应商 均不能为空', true); return;
      }
      try {
        await api('POST', actionUrl('upsert_model'), body);
        await refreshState(); rebuild(); changed();
        c.setMsg('已保存：' + body.id);
      } catch (e) { c.setMsg(e.message, true); }
    };

    rebuild(); rebuildDiscover();
    c.body.appendChild(listWrap);
    const form = document.createElement('div'); form.className = 'form-col';
    form.appendChild(field('id', idIn));
    form.appendChild(field('展示名', nameIn));
    form.appendChild(field('api_model', apiIn));
    form.appendChild(field('供应商', providerSel));
    form.appendChild(field('note', noteIn));
    // numberInput 返回 { input, wrap }，而 wrap **已经带标签**——直接把 wrap 挂上去
    //（传对象给 field 会让 appendChild 抛异常，整个表单就建不起来）。
    form.appendChild(ctxIn.wrap);
    form.appendChild(save);
    c.body.appendChild(form);
    const discTitle = document.createElement('div'); discTitle.className = 'wf-label'; discTitle.textContent = '从供应商获取模型（点选填入 api_model）';
    c.body.appendChild(discTitle);
    c.body.appendChild(discWrap);
}

function openModelsModal() {
  openModal('模型登记', modelsPage);
}

/* ---------- 设置③：核心 AI 默认模型 ---------- */
function coreModelBlock(c) {
    const opts = state.models.map((m) => ({ value: m.id, label: m.name + '（' + m.id + '）' }));
    const sel = selectInput(opts, state.core);
    const save = btn('保存', 'btn btn-primary btn-block');
    save.onclick = async () => {
      const id = sel.value;
      if (!id) { c.setMsg('请先登记模型', true); return; }
      try {
        await api('POST', actionUrl('set_core_model'), { id });
        await refreshState();
        c.setMsg('核心 AI 默认模型：' + id);
      } catch (e) { c.setMsg(e.message, true); }
    };
    if (!opts.length) c.body.appendChild(emptyHint('（未登记模型，先到「模型登记」添加）'));
    c.body.appendChild(field('核心 AI 使用的模型', sel));
    c.body.appendChild(save);
}

function openCoreModal() {
  openModal('核心 AI 默认模型', coreModelBlock);
}

/* ---------- 设置④：基本设置 ---------- */
/// 设置 1–4 页共用的控件：一次建全，切页只切显示——保存是全量写，不能因为没建过某页丢改动。
function settingsForm() {
  const stream = checkbox('流式传输（供应商逐片返回，边收边显示）', state.settings.streaming);
  const cot = checkbox('思维链显示（每条回答下的思维链，永远默认折叠、点击展开）', state.settings.show_reasoning);
  const to = numberInput('等待首个响应的超时（秒）', state.settings.llm_timeout_secs, 10, 3600);
  const remind = numberInput('一轮内最多提醒几次', state.settings.discuss_remind_cap, 0, 20);
  const pct = numberInput('上下文用到百分之多少就压缩', state.settings.compact_at_percent, 0, 100);
  const guests = numberInput('虚拟机档同时最多几台', state.settings.max_guests, 0, 64);
  const sb = state.settings.sandbox || {};
  const sp = sb.policy || {};
  const en = sp.enforce || {};
  const sd = sb.defaults || {};
  const lines = (v) => (Array.isArray(v) ? v.join('\n') : '');
  const area = (label, value) => {
    const t = areaInput('每行一条');
    t.value = lines(value);
    return { t, f: field(label, t) };
  };
  const network = checkbox('允许工具 / guest / 常驻服务 / 构建出站（全局）', !!sp.network);
  const writeAcl = checkbox('允许 Windows 写本机目录 ACL（只影响 Windows 的容器围栏）', !!(sp.windows && sp.windows.write_acl));
  const level = (v) => selectInput([
    { value: 'require', label: 'require：必须强制，不可用时询问是否本进程降级' },
    { value: 'best_effort', label: 'best_effort：尽力而为，不可用时降级并如实报告' },
  ], v || 'require');
  const enFs = level(en.filesystem);
  const enNet = level(en.network);
  const enTree = level(en.process_tree);
  const enEnv = level(en.env_whitelist);
  const onUnavailable = selectInput([
    { value: 'ask', label: 'ask：启动时询问是否本进程降级（不落盘）' },
    { value: 'refuse', label: 'refuse：直接拒绝' },
  ], sp.on_unavailable || 'ask');
  const hostRoots = area('主机只读根（绝对路径，每行一条）', sp.host_read_roots);
  const hardDeny = area('全局黑名单（工作区相对路径，只增不减）', sp.deny);
  const defRead = area('默认读白名单（会话可覆盖）', sd.allow_read);
  const defWrite = area('默认提交白名单（会话可覆盖）', sd.allow_write);
  const defAsk = area('默认 ask 表（会话可覆盖）', sd.ask);
  const defGran = selectInput([
    { value: 'ask', label: 'ask' },
    { value: 'full', label: 'full' },
  ], sd.granularity || 'ask');
  return { stream, cot, to, remind, pct, guests, network, writeAcl, enFs, enNet, enTree, enEnv, onUnavailable, hostRoots, hardDeny, defRead, defWrite, defAsk, defGran };
}

/// 1 页：生成与上下文。
function buildGeneratePage(c, f) {
  c.body.appendChild(f.stream.wrap);
  c.body.appendChild(f.cot.wrap);
  c.body.appendChild(f.to.wrap);
  c.body.appendChild(cfgHint('这是“等第一个响应/第一个数据块”的预算，不是整次流的总时长：正常流式只要还在出数据就不会超时；流开始后 120 秒没有新数据才判定断流。用尽时会中断本轮并提示，点「继续」可重试（会话不会作废）。'));
  c.body.appendChild(f.remind.wrap);
  c.body.appendChild(cfgHint('调用次数没有上限：模型继续核实就继续跑，直到它给出表态（你可以随时点「停止」）。'));
  c.body.appendChild(cfgHint('一轮内提醒到顶就记一行「未回应」放过它，整轮继续（不阻塞）。'));
  c.body.appendChild(f.pct.wrap);
  c.body.appendChild(cfgHint('压缩由 AI 自己做：到点自动压一次；也可以随时手动 /compact。填 0 = 不自动压。'));
}

/// 2 页：沙箱围栏（全局硬约束）。
function buildSandboxPage(c, f) {
  c.body.appendChild(cfgHint('以下沙箱策略是全局硬约束：会话不能改；保存后自动整体重启。', 'warn'));
  c.body.appendChild(f.network.wrap);
  c.body.appendChild(field('文件系统围栏', f.enFs));
  c.body.appendChild(field('网络围栏（仅在网络关闭时需要）', f.enNet));
  c.body.appendChild(field('进程树围栏', f.enTree));
  c.body.appendChild(field('环境白名单', f.enEnv));
  c.body.appendChild(field('机制不可用时', f.onUnavailable));
  c.body.appendChild(f.writeAcl.wrap);
  c.body.appendChild(cfgHint('Windows 上 require 文件系统/网络围栏必须先勾选「允许 Windows 写本机目录 ACL」；否则保存会被拒绝。要无围栏请把对应 enforce 降为 best_effort。', 'warn'));
}

/// 3 页：权限默认值。
function buildPermissionsPage(c, f) {
  c.body.appendChild(f.hostRoots.f);
  c.body.appendChild(f.hardDeny.f);
  c.body.appendChild(cfgHint('默认权限是会话可覆盖的起点；全局黑名单是硬底线，会话只能追加、不能清空。'));
  c.body.appendChild(f.defRead.f);
  c.body.appendChild(f.defWrite.f);
  c.body.appendChild(field('默认决定粒度', f.defGran));
  c.body.appendChild(f.defAsk.f);
}

/// 4 页：资源与进程。
function buildResourcesPage(c, f) {
  c.body.appendChild(f.guests.wrap);
  c.body.appendChild(cfgHint('每台虚拟机固定 2 GB 内存 / 2 核：到上限时新建会如实拒绝，不会悄悄抢资源。填 0 = 不限制。'));
  const title = document.createElement('div');
  title.className = 'wf-label';
  title.textContent = '现在在跑什么';
  const box = document.createElement('div');
  box.className = 'cfg-log';
  box.textContent = '读取中…';
  c.body.appendChild(title);
  c.body.appendChild(box);
  api('GET', '/api/processes').then((r) => {
    const list = (r && r.procs) || [];
    box.textContent = list.length ? list.map((p) =>
      (p.alive ? '在跑' : '已退出（等对账清）') + ' · ' + p.purpose + ' · ' + p.owner + ' · pid ' + p.pid +
      (p.orphanRisk ? ' · 父死子亡未挂上（核心被强杀会留下）：' + (p.guardNote || '') : '')
    ).join('\n') : '没有产品起的长驻进程。';
  }).catch((e) => { box.textContent = '读不到：' + e.message; });
}

function parseSettingLines(text) {
  return String(text || '').split(/[\n,]/).map((s) => s.trim()).filter((s) => s.length > 0);
}

/// 1–4 页共用一份提交体（set_settings 是全量写）。
function collectSettings(f) {
  return {
    streaming: f.stream.box.checked,
    show_reasoning: f.cot.box.checked,
    llm_timeout_secs: Number(f.to.input.value) || 300,
    discuss_remind_cap: Number(f.remind.input.value) || 0,
    compact_at_percent: Number(f.pct.input.value) || 0,
    max_guests: Number(f.guests.input.value) || 0,
    sandbox: {
      policy: {
        network: f.network.box.checked,
        enforce: {
          filesystem: f.enFs.value,
          network: f.enNet.value,
          process_tree: f.enTree.value,
          env_whitelist: f.enEnv.value,
        },
        on_unavailable: f.onUnavailable.value,
        windows: { write_acl: f.writeAcl.box.checked },
        host_read_roots: parseSettingLines(f.hostRoots.t.value),
        deny: parseSettingLines(f.hardDeny.t.value),
      },
      defaults: {
        allow_read: parseSettingLines(f.defRead.t.value),
        allow_write: parseSettingLines(f.defWrite.t.value),
        granularity: f.defGran.value,
        ask: parseSettingLines(f.defAsk.t.value),
      },
    },
  };
}

async function saveSettings(c, f, saveBtn) {
  const reset = busyButton(saveBtn, '保存中…');
  try {
    const saved = await api('POST', actionUrl('set_settings'), collectSettings(f));
    await refreshState();
    c.setMsg(saved && saved.restart ? '已保存：沙箱策略变更，正在重启…' : '已保存');
    ui.toast(saved && saved.restart ? 'warn' : 'ok', saved && saved.restart ? '已保存，正在重启…' : '设置已保存');
  } catch (e) {
    c.setMsg(e.message, true);
  } finally {
    reset();
  }
}

function openSettingsModal() {
  openSettingsHub('generate');
}

/* ---------- 设置中心：左侧分页、右侧单页、底栏统一保存 ----------
 * 1–4 页共享同一份 AppSettings（set_settings 是全量写）：控件一次建全，切页只切显示。
 * 5–8 页是即时的登记 / 启停操作，不显示保存栏。 */
const SETTINGS_PAGES = [
  { id: 'generate', group: '设置', label: '生成与上下文', icon: 'cpu' },
  { id: 'sandbox', group: '设置', label: '沙箱围栏', icon: 'shield' },
  { id: 'permissions', group: '设置', label: '权限默认值', icon: 'gear' },
  { id: 'resources', group: '设置', label: '资源与进程', icon: 'dot' },
  { id: 'providers', group: '登记', label: '供应商', icon: 'box' },
  { id: 'models', group: '登记', label: '模型与核心默认', icon: 'dot' },
  { id: 'agents', group: '登记', label: 'agent 管理', icon: 'users' },
  { id: 'modules', group: '登记', label: '模块与服务', icon: 'folder' },
];

function openSettingsHub(page) {
  openModal('设置', (c) => {
    const hub = document.createElement('div');
    hub.className = 'settings-hub';
    const nav = document.createElement('nav');
    nav.className = 'settings-nav';
    nav.setAttribute('aria-label', '设置分页');
    const panes = document.createElement('div');
    panes.className = 'settings-panes';
    const foot = document.createElement('div');
    foot.className = 'settings-foot';
    const form = settingsForm();
    const save = btn('保存', 'btn btn-primary');
    save.onclick = () => saveSettings(c, form, save);
    const hint = document.createElement('span');
    hint.className = 'settings-foot-hint';
    hint.textContent = '这四页共用同一份保存（全量写入）';
    foot.appendChild(hint);
    foot.appendChild(save);
    const main = document.createElement('div');
    main.className = 'settings-main';
    main.appendChild(panes);
    main.appendChild(foot);
    hub.appendChild(nav);
    hub.appendChild(main);

    const built = new Map();
    const ctxOf = (pane) => ({ body: pane, setMsg: c.setMsg });
    const builders = {
      generate: (pane, ctx) => buildGeneratePage(ctx, form),
      sandbox: (pane, ctx) => buildSandboxPage(ctx, form),
      permissions: (pane, ctx) => buildPermissionsPage(ctx, form),
      resources: (pane, ctx) => buildResourcesPage(ctx, form),
      providers: (pane, ctx) => providersPage(ctx),
      models: (pane, ctx) => {
        // 核心默认可选项必须在登记/删除模型后立即刷新（否则要整页刷新才看得到新模型）。
        const core = document.createElement('div');
        core.className = 'settings-core';
        pane.appendChild(core);
        const coreCtx = { body: core, setMsg: ctx.setMsg };
        const refreshCore = () => { core.innerHTML = ''; coreModelBlock(coreCtx); };
        refreshCore();
        ctx.afterChange = refreshCore;
        modelsPage(ctx);
      },
      agents: (pane, ctx) => agentsPage(ctx),
      modules: (pane, ctx) => modulesPage(ctx),
    };
    const settingsIds = new Set(['generate', 'sandbox', 'permissions', 'resources']);

    let group = '';
    for (const p of SETTINGS_PAGES) {
      if (p.group !== group) {
        group = p.group;
        const g = document.createElement('div');
        g.className = 'settings-nav-group';
        g.textContent = group;
        nav.appendChild(g);
      }
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'settings-nav-item';
      b.dataset.id = p.id;
      b.appendChild(ui.icon(p.icon));
      const label = document.createElement('span');
      label.textContent = p.label;
      b.appendChild(label);
      b.onclick = () => show(p.id);
      nav.appendChild(b);
    }

    function show(id) {
      if (!builders[id]) id = 'generate';
      for (const b of nav.children) {
        if (!b.dataset || !b.dataset.id) continue;
        b.className = 'settings-nav-item' + (b.dataset.id === id ? ' active' : '');
      }
      let pane = built.get(id);
      if (!pane) {
        pane = document.createElement('section');
        pane.className = 'settings-page';
        pane.dataset.id = id;
        panes.appendChild(pane);
        builders[id](pane, ctxOf(pane));
        built.set(id, pane);
      }
      for (const ch of panes.children) {
        ch.className = 'settings-page' + (ch.dataset.id === id ? '' : ' hidden');
      }
      foot.className = settingsIds.has(id) ? 'settings-foot' : 'settings-foot hidden';
      c.setMsg('');
    }

    c.body.appendChild(hub);
    show(page || 'generate');
  }, true);
}

/* ---------- 通用：多选一弹层（顶替原生 confirm 的三选） ---------- */
function choiceModal(title, question, actions) {
  openModal(title, (c) => {
    const q = document.createElement('div'); q.className = 'wf-label'; q.textContent = question;
    c.body.appendChild(q);
    const row = document.createElement('div'); row.className = 'wf-inline';
    for (const a of actions) {
      const b = btn(a[0], a[1] || 'btn');
      b.onclick = () => { closeModal(); a[2](); };
      row.appendChild(b);
    }
    c.body.appendChild(row);
  });
}

/* ---------- 设置⑤：agent 管理（列表 / 新建 / 编辑 / 删除） ---------- */
function agentsPage(c) {
    const listWrap = document.createElement('div'); listWrap.className = 'reg-list';
    const modWrap = document.createElement('div'); modWrap.className = 'wf-mods';
    const nameIn = textInput('agent 名字（会当沙箱目录名，非空、无非法字符、不能重名）');
    const modelSel = selectInput(state.models.map((m) => ({ value: m.id, label: m.name + '（' + m.id + '）' })), state.core);
    const noteIn = textInput('说明（这个 agent 干什么）');
    const save = btn('保存 agent', 'btn btn-primary btn-block');
    const cancelEdit = btn('取消编辑', 'btn btn-block hidden');
    let editing = null;   // 正在编辑的原始名字
    const picked = {};    // module id -> true

    function pickedIds() { return state.modules.map((m) => m.id).filter((id) => picked[id]); }
    function renderMods() {
      modWrap.innerHTML = '';
      if (!state.modules.length) { modWrap.appendChild(emptyHint('（modules/ 下没有模块）')); return; }
      for (const m of state.modules) {
        const lab = document.createElement('label'); lab.className = 'wf-check';
        const cb = document.createElement('input'); cb.type = 'checkbox'; cb.checked = !!picked[m.id];
        cb.addEventListener('change', () => { picked[m.id] = cb.checked; });
        const id = document.createElement('span'); id.textContent = m.id;
        const brief = document.createElement('span'); brief.className = 'wf-brief'; brief.textContent = m.brief;
        lab.appendChild(cb); lab.appendChild(id); lab.appendChild(brief);
        modWrap.appendChild(lab);
      }
    }
    function resetForm() {
      editing = null; nameIn.value = ''; noteIn.value = '';
      for (const k in picked) delete picked[k];
      cancelEdit.className = 'btn btn-block hidden';
      renderMods();
    }
    function renderList() {
      listWrap.innerHTML = '';
      if (!state.agents.length) { listWrap.appendChild(emptyHint('（还没有 agent）')); return; }
      for (const a of state.agents) {
        const row = document.createElement('div'); row.className = 'reg-item';
        const main = document.createElement('div'); main.className = 'reg-main';
        const nm = document.createElement('div'); nm.className = 'reg-id'; nm.textContent = a.name;
        const sub = document.createElement('div'); sub.className = 'reg-sub';
        sub.textContent = a.modules.join(' + ') + ' · 模型 ' + (a.model || '（核心默认）');
        const note = document.createElement('div'); note.className = 'reg-note'; note.textContent = a.note || '';
        main.appendChild(nm); main.appendChild(sub); main.appendChild(note);
        const acts = document.createElement('div'); acts.className = 'reg-acts';
        const edit = btn('编辑', 'link-btn');
        edit.onclick = () => {
          editing = a.name; nameIn.value = a.name; noteIn.value = a.note || '';
          if (a.model) modelSel.value = a.model;
          for (const k in picked) delete picked[k];
          for (const id of a.modules) picked[id] = true;
          cancelEdit.className = 'btn btn-block';
          renderMods(); c.setMsg('编辑 ' + a.name + '（改名 = 先删旧再建新）');
        };
        const del = btn('删除', 'link-btn danger');
        del.onclick = async () => {
          try {
            await api('POST', actionUrl('remove_agent'), { name: a.name });
            await refreshState(); renderList();
            if (editing === a.name) resetForm();
            c.setMsg('已删除 ' + a.name);
          } catch (e) { c.setMsg(e.message, true); }
        };
        acts.appendChild(edit); acts.appendChild(del);
        row.appendChild(main); row.appendChild(acts);
        listWrap.appendChild(row);
      }
    }
    cancelEdit.onclick = () => { resetForm(); c.setMsg(''); };
    save.onclick = async () => {
      const name = nameIn.value.trim();
      const modules = pickedIds();
      if (!name) { c.setMsg('agent 名字不能为空', true); return; }
      try {
        if (editing && editing !== name) {
          await api('POST', actionUrl('remove_agent'), { name: editing });
        }
        await api('POST', actionUrl('upsert_agent'), { name, modules, model: modelSel.value || undefined, note: noteIn.value.trim() });
        await refreshState(); renderList(); resetForm();
        c.setMsg('已保存 agent：' + name);
      } catch (e) { c.setMsg(e.message, true); }
    };

    renderList(); renderMods();
    c.body.appendChild(listWrap);
    const form = document.createElement('div'); form.className = 'form-col';
    form.appendChild(field('名字', nameIn));
    form.appendChild(field('模块（这个 agent 的能力）', modWrap));
    if (state.models.length) form.appendChild(field('默认模型', modelSel));
    form.appendChild(field('说明', noteIn));
    form.appendChild(save); form.appendChild(cancelEdit);
    c.body.appendChild(form);
}

function openAgentsModal() {
  openModal('agent 管理', agentsPage, true);
}

/* ---------- 上传文件到本次工作的 work/ ---------- */
const uploadPicker = document.createElement('input');
uploadPicker.type = 'file';
uploadPicker.addEventListener('change', () => {
  const f = uploadPicker.files && uploadPicker.files[0];
  if (!f) return;
  const s = activeSession();
  if (!s) { notice('还不能上传', '先打开或新建一个工作，再上传文件到它的 work/。'); uploadPicker.value = ''; return; }
  const reader = new FileReader();
  reader.onload = async () => {
    const b64 = String(reader.result || '').split(',')[1] || '';
    try {
      const r = await api('POST', actionUrl('upload'), { session_id: s.sid, name: f.name, data_base64: b64 });
      if (r.uploaded === false) {
        conflictUpload(s.sid, f.name, b64);
      } else {
        filesCache.delete(s.sid); // 文件清单变了，@ 菜单下次重拉
        ui.toast('ok', '已上传到 work/：' + f.name);
      }
    } catch (err) {
      notice('操作失败', err.message, 'err');
    }
    uploadPicker.value = '';
  };
  reader.readAsDataURL(f);
});

function pickUploadFile() { uploadPicker.click(); }

/* a.txt -> a-2.txt（用于「改名」建议） */
function suggestAltName(name) {
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return name + '-2';
  return name.slice(0, dot) + '-2' + name.slice(dot);
}

async function sendUpload(sid, name, b64, overwrite) {
  try {
    const body = { session_id: sid, name, data_base64: b64 };
    if (overwrite) body.overwrite = true;
    const r = await api('POST', actionUrl('upload'), body);
    if (r.uploaded === false) {
      notice('同名文件已存在', '目标里已经有这个名字，请改名或确认覆盖。', 'err');
      return;
    }
    filesCache.delete(sid); // 文件清单变了，@ 菜单下次重拉
    ui.toast('ok', '已上传：' + name);
  } catch (e) { notice('操作失败', e.message, 'err'); }
}

function conflictUpload(sid, name, b64) {
  const alt = suggestAltName(name);
  choiceModal('文件同名', '本次工作的 work/ 里已有「' + name + '」，怎么办？', [
    ['覆盖', 'btn btn-danger', () => sendUpload(sid, name, b64, true)],
    ['改名…', 'btn', () => renameUpload(sid, name, b64, alt)],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

function renameUpload(sid, name, b64, alt) {
  openModal('改文件名上传', (c) => {
    const inp = textInput('新文件名'); inp.value = alt;
    const go = btn('上传', 'btn btn-primary btn-block');
    go.onclick = async () => {
      const v = inp.value.trim();
      if (!v) { c.setMsg('文件名不能为空', true); return; }
      try {
        const r = await api('POST', actionUrl('upload'), { session_id: sid, name: v, data_base64: b64 });
        if (r.uploaded === false) { c.setMsg('还是同名，请换一个名字', true); return; }
        filesCache.delete(sid); // 文件清单变了，@ 菜单下次重拉
        closeModal(); ui.toast('ok', '已上传：' + v);
      } catch (e) {
        c.setMsg(e.message, true);
      }
    };
    c.body.appendChild(field('文件名', inp));
    c.body.appendChild(go);
  });
}
/* ---------- 新建工作向导 ---------- */
async function openWizard(draft) {
  // draft = 上一次没提交的草稿：补齐依赖后要把向导按原样重开，别把用户已经填的东西丢掉。
  // onclick 直接挂本函数时第一个参数是事件对象，所以按**标记**认草稿，不按真值认。
  const resumed = !!(draft && draft.__wizardDraft);
  // single 形态：w.modules = 勾选的模块；w.agentPick = 复用的登记处 agent
  // （null = 用勾选的模块组临时 agent，名字见 w.agentName）。
  const w = resumed ? draft : {
    mode: 'single', modules: [], agentPick: null, agentName: '', agents: [], task: '',
    tier: 'host', guest: '', __wizardDraft: true,
  };
  const ed = { open: false, name: '', modules: [], model: null, note: '' };
  // 名称与需求是**弹层里**输入框的现值（在本作用域之外）：由弹层装好后把取值钩子挂上来。
  let snapshotDraft = () => {};
  // 档位选择：默认档与虚拟机档可用性由后端给（GET /api/tiers，与「开始」的校验同源）。
  let tiers = null;
  try { const r = await api('GET', '/api/tiers'); tiers = (r && r.tiers) || null; } catch (e) { tiers = null; }
  if (!resumed && tiers && tiers.default) w.tier = tiers.default;
  // 档位那一格：虚拟机档不可用时禁用，并逐项说明缺什么、怎么补（照抄后端事实，不自己编话）。
  const tierField = document.createElement('div'); tierField.className = 'wf-field';
  const tierLabel = document.createElement('div'); tierLabel.className = 'wf-label'; tierLabel.textContent = '执行档位';
  const hostR = cfgRadio('本机档 —— 脚本直接在宿主上跑：宿主自备解释器；隔离就是宿主本身（默认）。', w.tier !== 'vm', 'wf-tier');
  const vmOk = !!(tiers && tiers.vm_available);
  const vmR = cfgRadio('虚拟机档 —— 一整套 guest，隔离更强；按模块声明装载运行包，依赖 guest 本体。', w.tier === 'vm', 'wf-tier', !vmOk);
  hostR.box.addEventListener('change', () => { if (hostR.box.checked) { w.tier = 'host'; syncWizardTier(); } });
  vmR.box.addEventListener('change', () => { if (vmR.box.checked) { w.tier = 'vm'; syncWizardTier(); } });
  tierField.appendChild(tierLabel); tierField.appendChild(hostR.wrap); tierField.appendChild(vmR.wrap);
  if (!vmOk) {
    const reqs = (tiers && tiers.vm_requirements) || [];
    // 只列**现状**：怎么补这件事由下面的按钮承担（把指引和动作分成两件事，用户不会只看到话看不到路）。
    const lines = reqs.filter((r) => !r.met).map((r) => '· ' + r.detail);
    tierField.appendChild(cfgHint(
      '虚拟机档现在不能选：' + ((tiers && tiers.vm_unavailable_reason) || '本机不具备虚拟机档的前置条件') +
      (lines.length ? '\n' + lines.join('\n') : ''),
      'err'
    ));
    const setupBtn = btn('补齐虚拟机依赖…', 'btn btn-block');
    // 装完把向导按草稿重开：判据是后端按新状态算的，界面不自己改结论。
    // 先把用户填的名称与需求抄进草稿（它们只在弹层里读得到），否则重开会把它们清掉。
    setupBtn.onclick = () => {
      // 兜住同步异常：按钮点了**必须**有反应——静默什么都不发生是最难查的失败。
      try {
        snapshotDraft();
        openSetupModal(() => openWizard(w));
      } catch (e) { notice('打开补齐弹层失败', e.message, 'err'); }
    };
    tierField.appendChild(setupBtn);
  }

  // 虚拟机档在**创建时**就要选一个虚拟根（会话级选择；机器能力已在上面判过）。
  const pick = guestPicker(tiers && tiers.guests, w.guest, (v, g) => { w.guest = v; w.guestInfo = g; });
  // 预判：这个虚拟根（+ 包库）满足不了本次点到的哪些模块——**只是提示**，判据仍以创建时的后端校验为准。
  const gapHint = cfgHint('');
  function syncGuestGap() {
    if (w.tier !== 'vm') { gapHint.textContent = ''; return; }
    const g = w.guestInfo;
    if (!g || !g.ready) { gapHint.textContent = ''; return; }
    const covered = {};
    (g.runtimes || []).forEach((c) => { covered[c] = true; });
    Object.keys((tiers && tiers.available) || {}).forEach((c) => { covered[c] = true; });
    const ids = w.mode === 'collab'
      ? w.agents.reduce((acc, a) => acc.concat(a.modules || []), [])
      : (w.agentPick ? (w.agentPick.modules || []).slice()
        : state.modules.map((m) => m.id).filter((id) => w.modules.indexOf(id) >= 0));
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
    syncGuestGap();
  }
  openModal('新建工作', (c) => {
    const nameIn = textInput('工作名称（必填，会话落盘目录名）');
    if (w.name) nameIn.value = w.name;   // 草稿重开：把用户填过的带回来
    const modeSel = selectInput([
      { value: 'single', label: '单 agent（1 个或多个模块）' },
      { value: 'collab', label: '协作（多个 agent）' },
      { value: 'proxy', label: '代理（把决定权整块交给核心）' },
    ], 'single');
    // 形态说明随选择走（三种形态的差别就在这一句里说清）。
    const modeHint = document.createElement('div'); modeHint.className = 'wf-hint';
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
    const partWrap = document.createElement('div'); partWrap.className = 'wf-field';
    const modelWrap = document.createElement('div'); modelWrap.className = 'wf-models';
    const editorWrap = document.createElement('div'); editorWrap.className = 'wf-field hidden';
    const taskIn = areaInput('本次需求（协作必填；也可写上，核心据此推荐）');
    if (w.task) taskIn.value = w.task;   // 草稿重开：同上
    // 上面那个「补齐虚拟机依赖」按钮在本作用域之外：把抄草稿的钩子交给它，点击时才读现值。
    snapshotDraft = () => { w.name = nameIn.value; w.task = taskIn.value; };
    // 需求那一格与推荐按钮都是**非代理形态**才问的：形态一换就收起来（见 renderParts）。
    const taskField = field('本次需求', taskIn);
    const recBtn = btn('让核心推荐', 'btn btn-block');
    const createBtn = btn('创建并开始', 'btn btn-primary btn-block');
    const cancelBtn = btn('取消', 'btn btn-block');
    cancelBtn.onclick = closeModal;

    const modelOptions = () => state.models.map((m) => ({ value: m.id, label: m.name + '（' + m.id + '）' }));

    // 勾选的模块（按清单顺序输出，下发稳定）。
    function pickedModules() {
      return state.modules.map((m) => m.id).filter((id) => w.modules.indexOf(id) >= 0);
    }

    // 复用候选 = 登记处全部 agent；额外把"核心推荐复用"的那条摆进来（否则选中项在列表里看不见）。
    function agentCandidates() {
      const list = state.agents.slice();
      if (w.agentPick && !list.some((a) => a.name === w.agentPick.name)) {
        list.push({ name: w.agentPick.name, modules: w.agentPick.modules.slice(), model: w.agentPick.model, note: '' });
      }
      return list;
    }

    // 选中/取消选中登记处 agent：模块由它决定（有几个勾几个），模型预填它的默认值（仍可改，只对本工作生效）。
    function pickAgent(name) {
      if (!name) { w.agentPick = null; renderAll(); return; }
      const a = agentCandidates().find((x) => x.name === name);
      if (!a) { w.agentPick = null; renderAll(); return; }
      w.agentPick = { name: a.name, modules: (a.modules || []).slice(), model: a.model || null };
      w.modules = w.agentPick.modules.slice();
      w.model = w.agentPick.model || state.core || w.model;
      renderAll();
    }

    function singlePick() {
      const box = document.createElement('div'); box.className = 'form-col';
      const opts = [{ value: '', label: '（不选，用下面勾选的模块组一个临时 agent）' }].concat(
        agentCandidates().map((a) => ({
          value: a.name,
          label: a.name + ' · ' + (a.modules || []).join('+') + ' · ' + (a.model || '核心默认'),
        })));
      const sel = selectInput(opts, w.agentPick ? w.agentPick.name : '');
      sel.addEventListener('change', () => pickAgent(sel.value));
      box.appendChild(field('agent（复用登记处已有的 agent）', sel));
      if (w.agentPick) {
        const hint = document.createElement('div'); hint.className = 'wf-hint';
        hint.textContent = '已由 agent「' + w.agentPick.name + '」决定模块与默认模型；' +
          '模型改动只对本工作生效，不写回登记处。';
        box.appendChild(hint);
      } else {
        const nm = textInput('agent 名字（默认取勾选的第一个模块）');
        nm.value = w.agentName || pickedModules()[0] || '';
        nm.addEventListener('input', () => { w.agentName = nm.value; });
        box.appendChild(field('本次 agent 的名字', nm));
      }
      const lab = document.createElement('div'); lab.className = 'wf-label';
      lab.textContent = w.agentPick
        ? '模块（已由 agent 决定，共 ' + w.agentPick.modules.length + ' 个）'
        : '模块（勾 1 个 = 直连式，勾多个 = 组合式）';
      box.appendChild(lab);
      box.appendChild(singleModules());
      return box;
    }

    function singleModules() {
      const box = document.createElement('div'); box.className = 'wf-mods';
      if (!state.modules.length) { box.appendChild(emptyHint('（modules/ 下没有模块）')); return box; }
      for (const m of state.modules) {
        const lab = document.createElement('label'); lab.className = 'wf-check' + (w.agentPick ? ' disabled' : '');
        const cb = document.createElement('input'); cb.type = 'checkbox';
        cb.checked = w.modules.indexOf(m.id) >= 0;
        if (w.agentPick) cb.disabled = true;
        cb.addEventListener('change', () => {
          if (w.agentPick) return;
          if (cb.checked) { if (w.modules.indexOf(m.id) < 0) w.modules.push(m.id); }
          else w.modules = w.modules.filter((x) => x !== m.id);
          renderAll();
        });
        const id = document.createElement('span'); id.textContent = m.id;
        const brief = document.createElement('span'); brief.className = 'wf-brief'; brief.textContent = m.brief;
        lab.appendChild(cb); lab.appendChild(id); lab.appendChild(brief);
        box.appendChild(lab);
      }
      return box;
    }

    // 协作：agent 实例列表（名字可就地改，同名会让后端加尾号）
    function agentRows() {
      const list = document.createElement('div'); list.className = 'agent-list';
      if (!w.agents.length) { list.appendChild(emptyHint('（还没加入 agent）')); return list; }
      w.agents.forEach((a, i) => {
        const row = document.createElement('div'); row.className = 'agent-item';
        const main = document.createElement('div'); main.className = 'reg-main';
        const nm = document.createElement('input'); nm.className = 'field-input agent-name';
        nm.value = a.name; nm.autocomplete = 'off'; nm.placeholder = 'agent 名字';
        nm.addEventListener('input', () => { a.name = nm.value; });
        const sub = document.createElement('div'); sub.className = 'reg-sub';
        // 登记处里的 agent 与临时 agent 如实分开：临时的那条可以就地存下来。
        const badge = document.createElement('span');
        badge.className = 'agent-badge' + (a.transient ? ' is-transient' : '');
        badge.textContent = a.transient ? '临时（可保存为 agent）' : '已在登记处';
        const mods = document.createElement('span'); mods.className = 'agent-mods'; mods.textContent = a.modules.join(' + ');
        sub.appendChild(badge); sub.appendChild(mods);
        main.appendChild(nm); main.appendChild(sub);
        const acts = document.createElement('div'); acts.className = 'wf-inline';
        if (a.transient) {
          const keep = btn('存为 agent', 'link-btn');
          keep.onclick = async () => {
            if (!a.name.trim()) { c.setMsg('agent 名字不能为空', true); return; }
            try {
              await api('POST', actionUrl('upsert_agent'), { name: a.name.trim(), modules: a.modules.slice(), model: a.model || undefined, note: '' });
              await refreshState();
              a.transient = false; a.reuse = true;
              renderAll(); c.setMsg('已保存 agent：' + a.name.trim());
            } catch (e) { c.setMsg(e.message, true); }
          };
          acts.appendChild(keep);
        }
        const del = btn('移除', 'link-btn danger');
        del.onclick = () => { w.agents.splice(i, 1); renderAll(); };
        acts.appendChild(del);
        row.appendChild(main); row.appendChild(acts);
        list.appendChild(row);
      });
      return list;
    }

    // 内联新建 agent（不弹二级窗，避免丢掉向导状态）
    function renderEditor() {
      editorWrap.className = ed.open ? 'wf-field' : 'wf-field hidden';
      editorWrap.innerHTML = '';
      if (!ed.open) return;
      const nm = textInput('新 agent 名字'); nm.value = ed.name;
      nm.addEventListener('input', () => { ed.name = nm.value; });
      const mods = document.createElement('div'); mods.className = 'wf-mods';
      for (const m of state.modules) {
        const lab = document.createElement('label'); lab.className = 'wf-check';
        const cb = document.createElement('input'); cb.type = 'checkbox'; cb.checked = ed.modules.indexOf(m.id) >= 0;
        cb.addEventListener('change', () => {
          if (cb.checked) { if (ed.modules.indexOf(m.id) < 0) ed.modules.push(m.id); }
          else ed.modules = ed.modules.filter((x) => x !== m.id);
        });
        const id = document.createElement('span'); id.textContent = m.id;
        const brief = document.createElement('span'); brief.className = 'wf-brief'; brief.textContent = m.brief;
        lab.appendChild(cb); lab.appendChild(id); lab.appendChild(brief);
        mods.appendChild(lab);
      }
      const mopts = modelOptions();
      const msel = selectInput(mopts, ed.model || state.core);
      if (mopts.length) ed.model = msel.value;
      msel.addEventListener('change', () => { ed.model = msel.value; });
      const note = textInput('说明（可选）'); note.value = ed.note;
      note.addEventListener('input', () => { ed.note = note.value; });
      const addT = btn('加入本次（临时）', 'btn');
      addT.onclick = () => {
        if (!ed.name.trim()) { c.setMsg('agent 名字不能为空', true); return; }
        w.agents.push({ name: ed.name.trim(), transient: true, modules: ed.modules.slice(), model: ed.model, why: null });
        ed.open = false; renderAll(); c.setMsg('已加入临时 agent：' + ed.name.trim());
      };
      const addS = btn('保存为 agent 并加入', 'btn btn-primary');
      addS.onclick = async () => {
        if (!ed.name.trim()) { c.setMsg('agent 名字不能为空', true); return; }
        try {
          await api('POST', actionUrl('upsert_agent'), { name: ed.name.trim(), modules: ed.modules.slice(), model: ed.model || undefined, note: ed.note.trim() });
          await refreshState();
          w.agents.push({ name: ed.name.trim(), transient: false, modules: ed.modules.slice(), model: ed.model, why: null });
          ed.open = false; renderAll(); c.setMsg('已保存并加入 agent：' + ed.name.trim());
        } catch (e) { c.setMsg(e.message, true); }
      };
      const cancelEd = btn('取消', 'btn btn-ghost');
      cancelEd.onclick = () => { ed.open = false; renderAll(); };
      editorWrap.appendChild(field('名字', nm));
      editorWrap.appendChild(field('模块（能力）', mods));
      if (mopts.length) editorWrap.appendChild(field('默认模型', msel));
      editorWrap.appendChild(field('说明', note));
      const row = document.createElement('div'); row.className = 'wf-inline';
      row.appendChild(addT); row.appendChild(addS); row.appendChild(cancelEd);
      editorWrap.appendChild(row);
    }

    function agentPicker() {
      const box = document.createElement('div'); box.className = 'form-col';
      box.appendChild(agentRows());
      const addRow = document.createElement('div'); addRow.className = 'wf-inline';
      const opts = state.agents.map((a) => ({ value: a.name, label: a.name + '（' + a.modules.join('+') + '）' }));
      const sel = selectInput(opts, null);
      const add = btn('加入', 'btn');
      add.onclick = () => {
        const a = state.agents.find((x) => x.name === sel.value);
        if (!a) { c.setMsg('请选择已登记的 agent，或点下面「新建 agent」', true); return; }
        if (w.agents.some((x) => x.name === a.name)) { c.setMsg('该 agent 已经在本次工作里了', true); return; }
        w.agents.push({ name: a.name, transient: false, modules: a.modules.slice(), model: a.model || state.core, why: null });
        renderAll();
      };
      if (opts.length) { addRow.appendChild(sel); addRow.appendChild(add); }
      else addRow.appendChild(emptyHint('（还没有 agent，先去「设置 → agent 管理」新建）'));
      box.appendChild(addRow);
      const newBtn = btn('+ 新建 agent', 'btn btn-block');
      newBtn.onclick = () => {
        ed.open = true; ed.name = ''; ed.modules = []; ed.model = state.core; ed.note = '';
        renderAll();
      };
      box.appendChild(newBtn);
      box.appendChild(editorWrap);
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
      const lab = document.createElement('div'); lab.className = 'wf-label';
      lab.textContent = w.mode === 'single' ? '单 agent：复用已有 agent，或勾选它的模块'
        : 'agent（协作：可多个，各自独立沙箱）';
      partWrap.appendChild(lab);
      partWrap.appendChild(w.mode === 'single' ? singlePick() : agentPicker());
    }

    function renderModels() {
      modelWrap.innerHTML = '';
      if (w.mode === 'proxy') {
        modelWrap.appendChild(emptyHint('（代理会话走核心默认通道：模型按登记处的核心默认）'));
        return;
      }
      const opts = modelOptions();
      if (w.mode === 'single') {
        const who = w.agentPick ? w.agentPick.name : ((w.agentName || '').trim() || pickedModules()[0] || '');
        if (!who) { modelWrap.appendChild(emptyHint('先复用 agent 或勾选模块，再指定模型')); return; }
        if (!opts.length) { modelWrap.appendChild(emptyHint('（未登记模型，将用核心默认）')); return; }
        const row = document.createElement('div'); row.className = 'wf-model-row';
        const lab = document.createElement('span'); lab.className = 'wf-model-id'; lab.textContent = who;
        const cur = w.model || state.core || opts[0].value;
        w.model = cur;
        const sel = selectInput(opts, cur);
        sel.addEventListener('change', () => { w.model = sel.value; });
        row.appendChild(lab); row.appendChild(sel);
        modelWrap.appendChild(row);
        return;
      }
      if (!w.agents.length) { modelWrap.appendChild(emptyHint('先加入 agent，再指定模型')); return; }
      if (!opts.length) { modelWrap.appendChild(emptyHint('（未登记模型，将用核心默认）')); return; }
      for (const a of w.agents) {
        const row = document.createElement('div'); row.className = 'wf-model-row';
        const lab = document.createElement('span'); lab.className = 'wf-model-id'; lab.textContent = a.name;
        const cur = a.model || state.core || opts[0].value;
        a.model = cur;
        const sel = selectInput(opts, cur);
        sel.addEventListener('change', () => { a.model = sel.value; });
        row.appendChild(lab); row.appendChild(sel);
        if (a.why) { const why = document.createElement('div'); why.className = 'wf-why'; why.textContent = a.why; row.appendChild(why); }
        modelWrap.appendChild(row);
      }
    }

    function renderAll() { renderParts(); renderModels(); renderEditor(); syncGuestGap(); }

    modeSel.addEventListener('change', () => {
      w.mode = modeSel.value;
      ed.open = false;
      renderAll();
    });

    recBtn.onclick = async () => {
      if (w.mode === 'proxy') { c.setMsg('代理形态不需要名单：核心自己挑人', true); return; }
      const task = taskIn.value.trim();
      if (!task) { c.setMsg('先写下本次需求，核心才能据此推荐', true); return; }
      c.setMsg('核心根据需求推荐中…');
      try {
        // 核心返回 agent 草案：{ name, modules, model, why, reuse }。
        // reuse=true → 这条就是登记处里已有的 agent（transient:false，行上标「已在登记处」）；
        // reuse=false → 核心帮忙组装的临时 agent（transient:true，行上标「临时（可保存为 agent）」）。
        const r = await api('POST', '/api/suggest-models', { task, mode: w.mode });
        const rec = (r.agents || []).filter((a) => (a.modules || []).length);
        if (!rec.length) { c.setMsg('核心未给出可用建议', true); return; }
        const asItem = (a) => ({
          name: a.name || a.modules[0],
          transient: a.reuse !== true,
          reuse: a.reuse === true,
          modules: a.modules.slice(),
          model: a.model,
          why: a.why || null,
        });
        let note = '核心已给出建议（见下方，agent 名字与模型都可改）';
        if (w.mode === 'single') {
          const top = rec[0];
          const mods = (top.modules || []).slice();
          if (top.reuse === true) {
            // 复用登记处已有的 agent：不论几个模块，完整按它的 modules 勾好（不截断）。
            const who = top.name || mods[0];
            const reg = state.agents.find((x) => x.name === who);
            const regMods = reg && reg.modules.length ? reg.modules.slice() : mods;
            w.agentPick = { name: who, modules: regMods, model: top.model || (reg && reg.model) || null };
            w.modules = regMods.slice();
            w.model = w.agentPick.model || state.core || w.model;
            note = '核心建议复用已有 agent「' + who + '」（' + regMods.length + ' 个模块）；' +
              '模型只对本工作生效，不写回登记处';
          } else {
            w.agentPick = null;
            w.modules = mods.slice();
            w.agentName = top.name || mods[0];
            if (top.model) w.model = top.model;
            note = '核心建议按临时 agent「' + w.agentName + '」组队（' + mods.length + ' 个模块）';
          }
        } else {
          w.agents = rec.map(asItem);
        }
        renderAll();
        c.setMsg(note);
      } catch (e) { c.setMsg(e.message, true); }
    };

    // 真正下发（重名时先经 choiceModal 让用户裁决，绝不用原生 confirm）
    // 要等：虚拟机档在这一步里起 guest（拉 QEMU + 等 agent，首次几十秒到几分钟）——按钮转圈 + 说清在等什么。
    async function submit(body) {
      const release = busyButton(createBtn, '正在创建…');
      cancelBtn.disabled = true;
      c.setMsg(w.tier === 'vm'
        ? '正在创建：虚拟机档要先起一台 guest（首次可能要几十秒到几分钟），起来之后才开工…'
        : '正在创建…');
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
      const name = nameIn.value.trim();
      if (!name) { c.setMsg('工作名称必填', true); return; }
      const task = taskIn.value.trim();
      const vmGuest = (w.guest || '').trim();
      if (w.tier === 'vm' && !vmGuest) {
        c.setMsg('虚拟机档要先选一个虚拟机（虚拟根）：点上面的「补齐虚拟机依赖」装上，或换一个已装好的。', true);
        return;
      }
      let agents;
      if (w.mode === 'proxy') {
        // 代理：没有名单、没有需求——选这个形态就是**授予全权**（后端建 mode=proxy 的会话）。
        await submit({ name, mode: 'proxy', agents: [], tier: w.tier, guest: w.tier === 'vm' ? vmGuest : null });
        return;
      }
      if (w.mode === 'single') {
        if (w.agentPick) {
          const mods = (w.agentPick.modules || []).slice();
          agents = [{
            name: w.agentPick.name, transient: false, modules: mods,
            model: w.model || w.agentPick.model || state.core,
          }];
        } else {
          const mods = pickedModules();
          const nm = (w.agentName || '').trim();
          if (!nm && !mods.length) {
            c.setMsg('单 agent：填一个 agent 名字，或至少勾选 1 个模块', true); return;
          }
          agents = [{
            name: nm || mods[0], transient: true, modules: mods,
            model: w.model || state.core,
          }];
        }
      } else {
        if (!w.agents.length) { c.setMsg('请先加入至少 1 个 agent', true); return; }
        if (!task) { c.setMsg('协作模式必须填写本次需求', true); return; }
        agents = w.agents.map((a) => ({ name: a.name.trim(), transient: !!a.transient, modules: a.modules.slice(), model: a.model }));
        if (agents.some((a) => !a.name)) { c.setMsg('agent 名字不能为空', true); return; }
      }
      const names = agents.map((a) => a.name);
      const dup = names.filter((n, i) => names.indexOf(n) !== i)[0];
      const body = { name, mode: w.mode, agents, tier: w.tier };
      if (w.tier === 'vm') body.guest = vmGuest;
      if (task) body.task = task;
      if (dup) {
        choiceModal('agent 重名', '本次工作里有同名 agent「' + dup + '」。', [
          ['按现名继续（后端自动加尾号 -2…）', 'btn btn-primary', () => submit(body)],
          ['回去改名', 'btn btn-ghost', () => {}],
        ]);
        return;
      }
      await submit(body);
    };

    c.body.appendChild(field('工作名称', nameIn));
    c.body.appendChild(field('形态', modeSel));
    c.body.appendChild(modeHint);
    c.body.appendChild(tierField);
    c.body.appendChild(pick.wrap);
    c.body.appendChild(pick.hint);
    c.body.appendChild(gapHint);
    c.body.appendChild(partWrap);
    c.body.appendChild(modelWrap);
    c.body.appendChild(taskField);
    c.body.appendChild(recBtn);
    c.body.appendChild(createBtn);
    c.body.appendChild(cancelBtn);

    renderParts(); renderModels(); syncWizardTier();
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
  const cached = filesCache.get(sid);
  if (cached) shortPathRoots = buildRootList(cached.roots);
  renderTabs(); renderStream(true); renderGate(state.sessions.get(sid));
  const s = state.sessions.get(sid);
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
  s.readonly = false; // 回档后的会话是活动会话（历史回放被用户明确回档/继续）
  for (const ev of (events || [])) absorb(s, ev);
}

/* ---------- 事件吸收：统一把 SessionEvent 变成行 ---------- */
function absorb(s, ev) {
  switch (ev.type) {
    case 'notice': s.lines.push({ cls: 'sys', who: '', text: ev.text }); break;
    // 压缩：显示**分界 + 摘要**（旧内容仍在上方可查——它只是不再发给模型）。
    case 'compacted':
      s.lines.push({
        cls: 'sys compacted',
        who: '',
        text: '[压缩] 此前内容已压成摘要（不再发给模型，仍可查看）：\n' + ev.summary,
      });
      break;
    // 留档回档：折叠分界。内容保留在文件里，可恢复。
    case 'rewind':
      if (ev.mode !== 'archive') break;
      s.lines.push({
        cls: 'sys rewind',
        who: '',
        text: '[留档] 以下内容已折叠（保留在文件里，可恢复）',
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
  }];
}

/* ---------- 渲染 ---------- */
function renderAll() { renderTabs(); renderStream(); renderGate(activeSession()); syncComposer(activeSession()); }

/* 只读历史视图：不渲染输入区；只留「启用会话」。启用是后端动作，前端只渲染结果。 */
function renderReadonly(s) {
  const bar = $('#readonly-bar');
  if (!bar) return;
  bar.innerHTML = '';
  if (!s || !s.readonly) { bar.classList.add('hidden'); return; }
  bar.classList.remove('hidden');
  const note = document.createElement('span');
  note.className = 'readonly-note';
  note.textContent = s.run === 'closed' ? '历史回放（只读）· 已关闭' : '历史回放（只读）：启用后才能发送消息';
  bar.appendChild(note);
  if (s.run === 'closed') return;
  const b = document.createElement('button');
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
    const rowEl = document.createElement('div');
    rowEl.className = 'tab-row' + (row.depth ? ' sub' : ' root');
    for (const name of row.names) {
      const opened = state.sessions.has(name);
      const el = document.createElement('button');
      el.type = 'button';
      el.className = 'tab' + (name === state.activeSid ? ' active' : '') + (opened ? '' : ' ghost');
      el.title = name;
      el.setAttribute('aria-selected', name === state.activeSid ? 'true' : 'false');
      el.dataset.badge = tabPendingDeep(name) && name !== state.activeSid ? '1' : '0';
      const label = document.createElement('span');
      label.textContent = tabLabel(name);
      el.appendChild(label);
      if (tabBusy(name)) {
        const dot = document.createElement('span');
        dot.className = 'dot dot-ok tab-dot';
        el.appendChild(dot);
      }
      if (opened) {
        const close = document.createElement('span');
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
  doneBox = document.createElement('div'); doneBox.className = 'stream-done';
  liveBox = document.createElement('div'); liveBox.className = 'stream-live';
  typingNode = document.createElement('div'); typingNode.className = 'typing hidden'; typingNode.textContent = 'agent 工作中';
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
  const el = document.createElement('div');
  el.className = 'line ' + cls + (live ? ' streaming' : '');
  if (p.who) {
    const w = document.createElement('span'); w.className = 'who'; w.textContent = p.who; el.appendChild(w);
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
  const d = document.createElement('div');
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
  return parts
    .filter((p) => p.cls !== 'tool')
    .map((p) => p.reasoning)
    .filter(Boolean)
    .join('\n\n');
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
  const el = document.createElement('article');
  el.className = 'turn' + (head.cls === 'ok' ? ' ok' : '');
  el.dataset.reply = String(group.reply || '');

  const headEl = document.createElement('div');
  headEl.className = 'turn-head';
  const who = document.createElement('span');
  who.className = 'turn-who';
  who.textContent = head.who || head.speaker || '';
  headEl.appendChild(who);
  if (tools.length) {
    const failed = tools.filter((p) => p.tool && !p.tool.ok).length;
    const cnt = document.createElement('span');
    cnt.className = 'turn-count';
    cnt.textContent = '工具 ' + tools.length + (failed ? ' · ' + failed + ' 失败' : '');
    headEl.appendChild(cnt);
  }
  const preview = document.createElement('span');
  preview.className = 'turn-preview';
  preview.textContent = text.replace(/\s+/g, ' ').slice(0, 64);
  headEl.appendChild(preview);
  const toggle = ui.iconButton({ icon: 'chevron', label: '折叠这一轮', tip: '', cls: 'turn-toggle' });
  headEl.appendChild(toggle);
  el.appendChild(headEl);

  const body = document.createElement('div');
  body.className = 'turn-body';
  if (reasoning && state.settings.show_reasoning) {
    body.appendChild(reasoningBlock(reasoning, s, key + ':cot'));
  }
  if (text.trim()) {
    const t = document.createElement('div');
    t.className = 'turn-text';
    appendBody(t, head.cls, text);
    body.appendChild(t);
  }
  if (tools.length) {
    const groupEl = document.createElement('details');
    groupEl.className = 'fold turn-tools';
    foldBind(s, key + ':tools', groupEl);
    const failed = tools.filter((p) => p.tool && !p.tool.ok).length;
    const sum = document.createElement('summary');
    sum.textContent = '工具调用 · ' + tools.length + (failed ? '（' + failed + ' 失败）' : '');
    groupEl.appendChild(sum);
    const list = document.createElement('div');
    list.className = 'tool-list';
    for (const p of tools) list.appendChild(toolCard(p.tool, s, 'T' + p.id, ''));
    groupEl.appendChild(list);
    body.appendChild(groupEl);
  }
  el.appendChild(body);

  const foot = document.createElement('div');
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
  const el = document.createElement('article');
  el.className = 'node-card ' + (l.state === 'done' ? 'done' : 'started');
  const head = document.createElement('div');
  head.className = 'node-head';
  const dot = document.createElement('span');
  dot.className = 'node-dot';
  head.appendChild(dot);
  const title = document.createElement('span');
  title.className = 'node-title';
  title.textContent = '节点 · ' + (n.id || '（未命名）');
  head.appendChild(title);
  const badge = document.createElement('span');
  badge.className = 'node-badge';
  badge.textContent = l.state === 'done'
    ? '完成' + (l.rework ? ' · 第 ' + l.rework + ' 轮返工' : '')
    : '进行中';
  head.appendChild(badge);
  if (n.sid) {
    const go = document.createElement('button');
    go.type = 'button';
    go.className = 'node-open';
    go.title = '进入这个节点的子会话';
    go.appendChild(ui.icon('folder'));
    const label = document.createElement('span');
    label.textContent = '子会话';
    go.appendChild(label);
    go.onclick = () => openPath(n.sid);
    head.appendChild(go);
  }
  el.appendChild(head);
  if (n.assignee) {
    const sub = document.createElement('div');
    sub.className = 'node-sub';
    sub.textContent = '执行：' + n.assignee;
    el.appendChild(sub);
  }
  if (l.report) {
    const prev = document.createElement('div');
    prev.className = 'node-preview';
    prev.textContent = String(l.report).split('\n')[0].slice(0, 96);
    el.appendChild(prev);
    const d = document.createElement('details');
    d.className = 'fold node-report';
    foldBind(s, 'N' + (n.id || ''), d);
    const sum = document.createElement('summary');
    sum.textContent = '回报全文';
    d.appendChild(sum);
    const pre = document.createElement('pre');
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
  const el = document.createElement('article');
  el.className = 'turn live';
  const head = document.createElement('div');
  head.className = 'turn-head';
  const who = document.createElement('span');
  who.className = 'turn-who';
  who.textContent = (owner && owner.speaker) || (owner && owner.tool && owner.tool.speaker) || '';
  head.appendChild(who);
  el.appendChild(head);
  const body = document.createElement('div');
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
      ui.textWrap = document.createElement('div');
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
      ui.tools = document.createElement('details');
      ui.tools.className = 'fold turn-tools';
      const sum = document.createElement('summary');
      ui.tools.appendChild(sum);
      ui.toolList = document.createElement('div');
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
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run);
  else setTimeout(run, 0);
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
  const el = document.createElement('div');
  el.className = 'gate-card';
  const who = document.createElement('div');
  who.className = 'who';
  who.textContent = (p.envelope && p.envelope.name) || '';
  if (who.textContent) el.appendChild(who);
  // **谁在等、前面还排着几条**：队首之后还在等的那几张如实列出来（只有队首能答）。
  const waiting = p.waiting || [];
  if (waiting.length) {
    const wq = document.createElement('div');
    wq.className = 'queue';
    wq.textContent =
      '后面还排着 ' + waiting.length + ' 张（先答上面这张）：' +
      waiting.map((w) => ((w.envelope && w.envelope.name) || '') + ' 在等 ' + (w.title || '')).join('；');
    el.appendChild(wq);
  }
  const q = document.createElement('div');
  q.className = 'q';
  q.textContent = msg.title || '';
  el.appendChild(q);
  if (msg.body) {
    const bd = document.createElement('div');
    bd.className = 'hint';
    bd.textContent = msg.body;
    el.appendChild(bd);
  }
  if (msg.detail) {
    const dt = document.createElement('div');
    dt.className = 'advice';
    dt.textContent = msg.detail;
    el.appendChild(dt);
  }
  const inp = document.createElement('input');
  inp.className = 'decision-input';
  inp.placeholder = '附言（可空；要你回话 / 先说一句的那一关必填）…';
  inp.disabled = busy;
  el.appendChild(inp);
  const bs = document.createElement('div');
  bs.className = 'btns';
  for (const o of (p.options || [])) {
    const bt = document.createElement('button');
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
   服务端回到需求行并追加新需求，返回完整重放，前端整体重建。 */
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
  const el = document.createElement('div');
  el.className = 'gate-card';
  const qq = document.createElement('div'); qq.className = 'q'; qq.textContent = q; el.appendChild(qq);
  const bs = document.createElement('div'); bs.className = 'btns';
  for (const [label, fn] of btns) {
    const b = document.createElement('button'); b.className = 'btn'; b.textContent = label;
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
  choiceModal('回档到此处', '留档：保留内容、折叠成标记，可随时恢复；删除：真的删掉这一行及其后的全部，不可恢复。', [
    ['留档', 'btn', () => applyRewind(s, { id, mode: 'archive' })],
    ['删除', 'btn btn-danger', () => applyRewind(s, { id, mode: 'delete' })],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 恢复某次留档：删掉该标记及其后的内容（含留档期间的新工作）。 */
function restoreMark(mark) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  choiceModal('恢复这次留档', '恢复到留档之前：该标记及其后的全部内容（含留档期间的新工作）会被删除。', [
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
    renderAll();
    ui.toast('ok', '已回档');
  } catch (err) { notice('操作失败', err.message, 'err'); }
}

/* 撤回某 agent 的同意（转录追加撤回行，协作才有意义）；值 = agent 实例名。 */
function withdrawAgree(agent) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  choiceModal('撤回同意', '撤回「' + agent + '」的同意？继续时会按剩余转录重新判定。', [
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

/* ---------- 输入框 @ 引用（工作区 / 沙箱里的文件） ---------- */
/* 数据 = GET /api/sessions/{sid}/files，按会话缓存；上传成功即失效；切换会话即清菜单。
 * 菜单只列已存在的文件（上传仍走输入区那个 ＋ 按钮）；前端只负责**插入与渲染**，
 * 不做任何路径改写——@work:… / @sandbox:… 由核心在入转录前改写成 work:/… / sandbox:/…。 */
const filesCache = new Map();
const atState = { open: false, loading: false, items: [], all: [], active: 0, start: -1, sid: null };

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
  atState.active = 0; atState.start = -1; atState.sid = null;
  const box = atMenuEl();
  if (box) { box.className = 'at-menu hidden'; box.innerHTML = ''; }
}

/* 菜单底部的固定提示行（可发现性）。 */
function atHint(box) {
  const h = document.createElement('div'); h.className = 'at-hint';
  h.textContent = '↑↓ 选择 · Enter 填入 · Esc 关闭';
  box.appendChild(h);
}

async function atLoad(sid) {
  if (filesCache.has(sid)) return filesCache.get(sid);
  return atFetch(sid);
}

/* **每次打开 @ 菜单都重拉一次**：文件可能在会外被手工删掉，或被 agent 的工具改过。
 * 前端不引入目录 watcher（本机、跨平台，代价不值）；拉到的结果写回缓存，
 * 供长路径缩写（setActive / loadPathRoots）快速复用。 */
async function atFetch(sid) {
  try {
    const data = await api('GET', '/api/sessions/' + encodeURIComponent(sid) + '/files');
    filesCache.set(sid, data);
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
    const e = document.createElement('div'); e.className = 'at-empty'; e.textContent = '载入中…';
    box.appendChild(e);
    atHint(box);
    return;
  }
  if (!atState.items.length) {
    const e = document.createElement('div'); e.className = 'at-empty'; e.textContent = '（没有匹配的文件）';
    box.appendChild(e);
    atHint(box);
    return;
  }
  let group = null;
  let activeRow = null;
  atState.items.forEach((it, i) => {
    if (it.group !== group) {
      group = it.group;
      const g = document.createElement('div'); g.className = 'at-group'; g.textContent = group;
      box.appendChild(g);
    }
    const row = document.createElement('div');
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

/* 插入：把 @ 与已输入的前缀一起替换成完整引用，光标落在其后。 */
function atInsert(it) {
  const t = $('#input');
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

async function atOnInput() {
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
let pollSince = 0;
let pollActive = false;
async function pollLoop() {
  if (pollActive) return;
  pollActive = true;
  while (true) {
    // ① 取事件：**只有这一步失败才算断线**（服务没起来 / 连接断了）。
    let data;
    try {
      const r = await fetch('/api/events?sid=&since=' + pollSince);
      if (!r.ok) throw new Error('轮询失败 ' + r.status);
      data = await r.json();
    } catch (err) {
      setConn(false, err);
      await new Promise((res) => setTimeout(res, 3000));
      continue;
    }
    setConn(true);
    // ② 应用事件：出错是**界面自己的问题**，不能说成断线（连接明明好着），也不能吞掉。
    // 为什么必须分开：渲染里的一个异常曾被当成"网络断了"，于是状态点一直红着、
    // 而真正的异常连一行日志都没有——用户只看到"重连中"，服务其实好好的。
    try {
      const head = data.head != null ? data.head : pollSince;
      const oldest = typeof data.oldest === 'number' ? data.oldest : 0;
      // 事件台裁剪过：since 之后有一段**永久丢了**。按 seq 干等会让后续批次全部滞留
      // （只有刷新页面才恢复）——所以这里重新对齐：丢掉滞留，跳到还留着的起点，
      // 并把当前会话按历史重放一次。
      if (oldest > 0 && oldest > appliedSeq + 1) {
        appliedSeq = oldest - 1;
        // 事件台裁掉了一段：**运行态的实时知识作废**（收尾那条可能就在丢掉的那段里），
        // 退回"用快照对账"，直到事件重新告诉我们为止。
        for (const cur of state.sessions.values()) cur.running_known = false;
        needState = true;
        await resyncActive();
      }
      pollSince = head;
      let mode = 'none';
      for (const item of data.lines) {
        try {
          const m = applyBatch(item.seq, item.sid, item.events);
          if (m === 'full') mode = 'full';
          else if (m === 'live' && mode !== 'full') mode = 'live';
          // 运行态只在 applyBatch 里由事件改（`working`）——
    // 这里碰它，一个迟到的流式增量就会把「停止」按钮误翻回「发送」。
        } catch { /* 单行损坏不拖垮轮询 */ }
      }
      // 状态对齐：别的客户端（演示脚本、另一个标签页）建/删/改会话时，侧栏自己跟上——
      // 不需要用户刷新浏览器；未知会话会立刻置 needState，其余靠 3 秒兜底。
      if (needState || Date.now() - lastStateAt > 3000) {
        needState = false;
        lastStateAt = Date.now();
        // refreshState 自己会把侧栏与历史重画；agent 登记弹窗里的列表归那个弹窗自己管。
        await refreshState();
      }
      // 纯流式增量：只 append 新节点（折叠、<pre> 滚动、外层滚动都不被打断）。
      if (mode === 'full') renderAll();
      else if (mode === 'live') scheduleLiveTick();
      // 有会话动作在等回包时，让动作回包自己刷新 pending；轮询只补漏。
    } catch (err) {
      eventError(err);
    }
  }
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
function modulesPage(c) {
  const load = async () => {
    c.body.innerHTML = '';
    c.setMsg('加载中…');
    let mods = [];
    let services = [];
    let actions = [];
    try {
      const st = await api('GET', '/api/state');
      mods = st.modules || [];
      services = st.services || [];
      const ac = await api('GET', '/api/actions');
      // 隐秘字段的设置面另有入口：值不在这一块面板里回显。
      actions = (ac.actions || []).filter((a) => a.id !== 'set_secret' && a.id !== 'clear_secret');
    } catch (e) {
      c.setMsg('加载失败：' + e.message, true);
      return;
    }
    c.setMsg('');
    renderModulesPanel(c, mods, services, actions, load);
  };
  load();
}

function openModulesPanel() {
  openModal('模块与服务', modulesPage, true);
}

/** 纯渲染（数据注入）：模块 → 服务（开关 + 运行态）→ 功能（「使用」按钮）。 */
function renderModulesPanel(ctx, mods, services, actions, reload) {
  if (!mods.length) {
    ctx.body.appendChild(emptyHint('（modules/ 下没有模块）'));
    return;
  }
  for (const m of mods) {
    const sec = panelSection(m.id + (m.brief ? ' · ' + m.brief : ''));
    // 模块开关：停用会连带停掉它的常驻服务（核心侧级联）。
    const head = document.createElement('div');
    head.className = 'svc-row';
    const state = document.createElement('span');
    state.className = 'svc-state';
    state.textContent = m.enabled ? '整体启用' : '整体停用';
    head.appendChild(state);
    const toggle = btn(m.enabled ? '停用模块' : '启用模块', 'btn');
    toggle.onclick = () => fire(ctx, 'set_module_enabled', { module: m.id, enabled: !m.enabled }, reload);
    head.appendChild(toggle);
    sec.appendChild(head);

    // 服务：模块声明的常驻服务（声明在 modules[].services，运行态在 services）。
    sec.appendChild(cfgDiv('cfg-sec-title', '服务'));
    const declared = m.services || [];
    if (!declared.length) {
      sec.appendChild(emptyHint('（没有声明常驻服务）'));
    } else {
      for (const s of declared) {
        const rt = services.find((x) => x.module === m.id && x.name === s.name) || {};
        const row = document.createElement('div');
        row.className = 'svc-row';
        const name = document.createElement('span');
        name.className = 'svc-name';
        name.textContent = s.name + (rt.adapter ? ' · ' + rt.adapter : '') +
          ' · ' + (rt.state || (s.enabled ? '未启动' : '已禁用'));
        row.appendChild(name);
        const sw = btn(s.enabled ? '停用服务' : '启用服务', 'btn');
        sw.onclick = () => fire(ctx, 'control_resident',
          { module: m.id, name: s.name, action: s.enabled ? 'disable' : 'enable' }, reload);
        row.appendChild(sw);
        sec.appendChild(row);
      }
    }

    // 功能：本模块的工具与服务操作（每个一行；参数与运行在子卡片里）。
    sec.appendChild(cfgDiv('cfg-sec-title', '功能'));
    const acts = actions.filter((a) => a.id.indexOf('module.' + m.id + '.') === 0);
    if (!acts.length) {
      sec.appendChild(emptyHint('（只能经 AI 使用：没有可直跑的 tools / services）'));
    } else {
      for (const a of acts) sec.appendChild(useRow(a));
    }
    ctx.body.appendChild(sec);
  }
}

/** 功能一行：只显示名字与「使用」；参数与运行在子卡片里，列表不再平铺表单。 */
function useRow(action) {
  const row = document.createElement('div');
  row.className = 'svc-row';
  const name = document.createElement('span');
  name.className = 'svc-name';
  const short = action.id.replace(/^module\.[^.]+\./, '');
  name.textContent = short + (action.desc ? ' — ' + action.desc : '');
  row.appendChild(name);
  const use = btn('使用', 'btn');
  use.onclick = () => openUseCard(action);
  row.appendChild(use);
  return row;
}

/** 子卡片：一个功能的参数与运行（回执打在卡片里）。 */
function openUseCard(action) {
  const ctx = openModal(action.id, () => {});
  if (action.desc) ctx.setMsg(action.desc);
  ctx.body.appendChild(actionRunner(action));
}

/** 跑一条动作后再重绘面板：失败如实显示，成功按新状态重绘。 */
async function fire(ctx, id, args, reload) {
  try {
    await api('POST', actionUrl(id), args);
  } catch (e) {
    ctx.setMsg('操作失败：' + e.message, true);
    return;
  }
  await reload();
}

function panelSection(title) {
  const sec = document.createElement('div');
  sec.className = 'svc-sec';
  const h = document.createElement('div');
  h.className = 'svc-title';
  h.textContent = title;
  sec.appendChild(h);
  return sec;
}

/** 一条动作的运行块：参数按声明现渲染，运行走 /api/actions/{id}，回执打在下面。 */
function actionRunner(action) {
  const box = document.createElement('div');
  box.className = 'svc-run';
  const name = document.createElement('div');
  name.className = 'svc-name';
  name.textContent = action.id + (action.desc ? ' — ' + action.desc : '');
  box.appendChild(name);
  const inputs = {};
  for (const p of action.params || []) {
    const i = textInput(p.name + (p.required ? '（必填）' : ''));
    inputs[p.name] = { p, i };
    box.appendChild(field(p.name, i));
  }
  const out = document.createElement('pre');
  out.className = 'svc-out';
  const run = btn('运行', 'btn');
  run.onclick = async () => {
    const args = {};
    for (const k of Object.keys(inputs)) {
      const { p, i } = inputs[k];
      if (i.value === '' && !p.required) continue;
      if (p.ty === 'integer') args[k] = Number(i.value);
      else if (p.ty === 'boolean') args[k] = i.value === 'true';
      else args[k] = i.value;
    }
    out.textContent = '运行中…';
    try {
      const r = await api('POST', actionUrl(action.id), args);
      out.textContent = JSON.stringify(r, null, 2);
    } catch (e) {
      out.textContent = '失败：' + e.message;
    }
  };
  box.appendChild(run);
  box.appendChild(out);
  return box;
}

/* ---------- 抽屉遮罩 / 顶层接线 ---------- */
$('#drawer-mask').onclick = closeDrawer;
$('#btn-new-work').onclick = openWizard;
$('#btn-settings').onclick = () => openSettingsHub('generate');
$('#fence-settings').onclick = () => openSettingsHub('sandbox');
$('#btn-upload').onclick = pickUploadFile;
setupSidebar();

/* ---------- 启动 ---------- */
refreshState()
  .then(pollLoop)
  .catch((err) => notice('初始化失败', err.message, 'err'));
