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
  sessions: new Map(),   // sid -> { sid, mode, title, lines, pending, busy, done, awaiting, readonly }
  activeSid: null,
  settingsOpen: false,
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
  // 服务端的**权威会话视图**：在跑 / 有没有"本次需求" / 有没有等裁决。
  // 事件流给增量（推），这里是刷新后照样成立的快照（拉）——两面同一个事实。
  state.views = new Map((s.sessions || []).map((v) => [v.sid, v]));
  state.running = new Set(
    [].concat(
      (s.sessions || []).filter((v) => v.running).map((v) => v.sid),
      (state.history || []).filter((h) => h.running).map((h) => h.name),
    ),
  );
  // 已打开的标签页跟着快照对齐（增量事件到达时会覆盖成同一份）。
  for (const cur of state.sessions.values()) {
    const v = state.views.get(cur.sid);
    if (!v) continue;
    cur.can_update_task = !!v.can_update_task;
    cur.pending = v.pending || null;
  }
  state.settings = s.settings || { streaming: true, show_reasoning: true, llm_timeout_secs: 300, discuss_remind_cap: 3, compact_at_percent: 70 };
  renderSidebar();
  renderHistory();
  const cur = activeSession();
  if (cur) { syncTyping(cur); syncSendButton(cur); }
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

/// 兜底卡片：正文看起来是工具信封（旧会话里核心曾把它当发言落盘）→ 按工具卡片渲染。
/// 键用 L<行id>:raw，与真实工具调用的 T<序号>:… 分开，不串味。
function rawToolCard(text, sess, key, speaker) {
  const body = String(text || '');
  const mod = (body.match(/"module"\s*:\s*"([^"]*)"/) || [])[1] || '';
  const name = (body.match(/"name"\s*:\s*"([^"]*)"/) || [])[1] || '工具信封';
  const el = document.createElement('div');
  el.className = 'line tool-card bad';
  el.title = speaker ? speaker + ' 的工具信封（不完整）' : '工具信封（不完整）';
  const head = document.createElement('div'); head.className = 'tool-head';
  const nm = document.createElement('span'); nm.className = 'tool-name';
  nm.textContent = (mod ? mod + '.' : '') + name;
  const mark = document.createElement('span'); mark.className = 'tool-mark'; mark.textContent = '✗';
  const note = document.createElement('span'); note.className = 'tool-note'; note.textContent = '信封不完整';
  head.appendChild(nm); head.appendChild(mark); head.appendChild(note);
  el.appendChild(head);
  el.appendChild(toolDetail('原文', body, sess, key + ':raw', true));
  return el;
}

/// 工具调用卡片：头部只写「模块.工具名」+ 成败；参数与结果折叠在里面（折叠状态走 fold store）。
/// 流式 tool_call 与权威 transcript 的 tool 行共用这一个渲染函数，也共用同一个折叠键。
function toolCard(t, sess, key) {
  const info = t || {};
  const el = document.createElement('div');
  el.className = 'line tool-card ' + (info.ok ? 'ok' : 'bad');
  el.title = info.speaker ? info.speaker + ' 调用工具' : '工具调用';
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

/* ---------- 会话历史：只读回放；每项只有「打开」「编辑」两个动作 + 删除 ----------
 * 点条目本身不进入会话（避免误开）：进入一律走「打开」按钮。
 * 正在生成中的会话只禁用「编辑」（理由写在按钮 title 上）：后端也会再拦一次，这里只是先挡住。 */
function renderHistory() {
  const box = $('#history-list');
  if (!box) return;
  box.innerHTML = '';
  const items = state.history || [];
  if (!items.length) { box.textContent = '（还没有历史会话）'; return; }
  for (const h of items) {
    const el = document.createElement('div');
    // 子会话（parent 指向另一个会话）在侧栏里**缩进**挂在父会话下。
    el.className = 'history-item' + (h.parent ? ' history-child' : '');
    const name = document.createElement('span'); name.className = 'hname';
    name.textContent = (h.parent ? '└ ' : '') + h.name;
    const mode = document.createElement('span'); mode.className = 'hmode';
    mode.textContent =
      (h.done ? '' : '·进行中 ') + h.mode +
      (h.tier === 'vm' && h.tier_ready === false ? '·虚拟机档不可用' : '');
    const acts = document.createElement('div'); acts.className = 'history-acts';

    const open = btn('打开', 'hbtn');
    open.title = '进入会话视图（转録 + 继续 / 停止）';
    open.onclick = (e) => { e.stopPropagation(); openHistory(h.name); };

    const edit = btn('编辑', 'hbtn');
    if (generating(h.name)) {
      edit.disabled = true;
      edit.title = '该会话正在生成中：先「停止」或等它结束，再改配置';
    } else {
      edit.title = '打开配置面板（在进入会话视图之前改名单 / 模块 / 模型 / 档位）';
      edit.onclick = (e) => { e.stopPropagation(); openConfig(h.name); };
    }
    acts.appendChild(open); acts.appendChild(edit);

    const del = btn('✕', 'hdel');
    del.title = '删除该会话（记录永久删除）';
    del.onclick = (e) => { e.stopPropagation(); deleteHistory(h.name); };

    el.appendChild(name); el.appendChild(mode); el.appendChild(acts); el.appendChild(del);
    box.appendChild(el);
  }
}

/// 这条会话此刻在不在干活：**服务端权威运行态**（自己 or 它的任一子会话在跑）
/// + 本标签页的本地推断（自己发起的动作、正在到达的流式增量）。
/// 为什么要服务端那份：成员回合跑在它自己的会话里，主会话整回合收不到事件——
/// 只靠"有增量"猜就永远切不出「停止」按钮、也没有占位动画。
function isBusy(s) {
  if (!s) return false;
  if (s.busy) return true;
  const run = state.running;
  if (!run) return false;
  if (run.has(s.sid)) return true;
  const pre = s.sid + '--';
  for (const k of run) if (k.indexOf(pre) === 0) return true;
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
      '并在「编辑」里确认执行档位与基础根；暂时不需要就在「编辑」里改回本机档。',
    'warn'
  );
}

async function openHistory(name) {
  const existing = state.sessions.get(name);
  if (existing && !existing.readonly) { setActive(name); warnIfTierUnavailable(name); return; }
  try {
    const r = await api('GET', '/api/history/' + encodeURIComponent(name));
    const s = {
      sid: name, mode: (r.meta && r.meta.mode) || 'single', title: name,
      lines: [], live: [], pending: null, busy: false, done: true, awaiting: null, readonly: true, fold: {}, scroll: {},
    };
    state.sessions.set(name, s);
    for (const ev of (r.events || [])) absorb(s, ev);
    setActive(name);
    renderAll();
    warnIfTierUnavailable(name);
  } catch (err) { notice('操作失败', err.message, 'err'); }
}

async function deleteHistory(name) {
  if (!(await confirmBox('删除会话', '删除会话「' + name + '」？该会话的记录将被永久删除。', '删除'))) return;
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

/// 结构化诊断 → 中文（呈现层文案，不进提示词册）。
function diagnosisText(d) {
  if (!d || typeof d !== 'object') return String(d);
  if (d.Missing) {
    return '模块 ' + d.Missing.module + ' 需要的运行能力 ' + d.Missing.capability +
      ' 在包库里没有：把它放进依赖文件夹后点「重新扫描」。';
  }
  if (d.Ambiguous) {
    return '运行能力 ' + d.Ambiguous.capability + ' 有多个版本（' + (d.Ambiguous.versions || []).join('、') +
      '）：在下面「定版」里选一个版本，虚拟机档才装得起来。';
  }
  if (d.UnknownPin) {
    return '运行能力 ' + d.UnknownPin.capability + ' 定的版本 ' + d.UnknownPin.version +
      ' 不在包库里：改一个版本，或把该版本的运行包放进依赖文件夹后点「重新扫描」。';
  }
  if (d.Conflict) {
    return '运行包 ' + d.Conflict.a + ' 与 ' + d.Conflict.b + ' 都要写进 ' + d.Conflict.path +
      '：装配会互相覆盖，换版本或去掉其中一个。';
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
    base: cfg.base || '',
    pins: {},
    net: !!cfg.net,
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

  // agent 名单：名字 / 模块 / 模型
  const dupHints = [];
  const syncDup = () => {
    const owners = {};
    for (const a of draft.agents) {
      for (const id of a.modules) {
        owners[id] = owners[id] || [];
        owners[id].push(a.name.trim() || '（未命名）');
      }
    }
    dupHints.forEach((h, i) => {
      const mine = draft.agents[i].modules.filter((id) => owners[id].length > 1);
      h.textContent = mine.length
        ? '同一模块只能属于一个 agent：' + mine.join('、') + ' 同时出现在多个 agent 里，保存会被后端拒绝。'
        : '';
      h.className = 'cfg-hint' + (mine.length ? ' warn' : '');
    });
  };
  const secA = cfgDiv('cfg-sec');
  secA.appendChild(cfgDiv('cfg-sec-title', 'agent 名单（' + draft.agents.length + ' 个）'));
  if (!draft.agents.length) secA.appendChild(emptyHint('（这个会话没有 agent 记录）'));
  draft.agents.forEach((a, i) => secA.appendChild(cfgAgentBlock(a, i, frozen, dupHints, syncDup)));
  box.appendChild(secA);
  syncDup();

  // 执行档位：本机 / 虚拟机（如实说明各自是什么）
  const secT = cfgDiv('cfg-sec');
  secT.appendChild(cfgDiv('cfg-sec-title', '执行档位'));
  // 虚拟机档的**承载**由后端判定（与「开始」/保存同一把尺子）：前置条件不具备时禁用，不让选。
  // 与「未接入」是两回事：guest 本体尚未接入这一点写在下面的提示里，两条都如实说。
  const vmOk = cfg.vm_available !== false;
  const hostR = cfgRadio('本机档 —— 脚本直接在宿主上跑：宿主自备解释器，不装载运行包；隔离就是宿主本身（快，但风险也在宿主上）。', draft.tier === 'host', 'cfg-tier');
  const vmR = cfgRadio('虚拟机档 —— 一整套 guest，脚本在 guest 里跑：按模块声明的运行能力装载运行包；隔离更强，代价是更重、依赖运行包。', draft.tier === 'vm', 'cfg-tier', !vmOk);
  const baseIn = textInput('base（可选）：虚拟机的基础根（发行版基底名或目录）');
  baseIn.value = draft.base;
  baseIn.addEventListener('input', () => { draft.base = baseIn.value; });
  const baseWrap = field('虚拟机基础根 base（可选）', baseIn);
  const baseHint = cfgHint('虚拟机档才用得上；留空 = 用默认基底。本机档忽略它（提交时按 null 送出）。');
  const syncTier = () => {
    const vm = draft.tier === 'vm';
    baseWrap.className = vm ? 'wf-field' : 'wf-field hidden';
    baseHint.className = 'cfg-hint' + (vm ? '' : ' hidden');
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
  }
  secT.appendChild(baseWrap); secT.appendChild(baseHint);
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

  // 网络
  const secN = cfgDiv('cfg-sec');
  secN.appendChild(cfgDiv('cfg-sec-title', '网络'));
  const net = checkbox('放行出站网络（默认不放行：虚拟机 guest 无网卡，脚本连不出去；本机档不隔离网络，这个开关只对虚拟机档有意义）', draft.net);
  net.box.addEventListener('change', () => { draft.net = net.box.checked; });
  secN.appendChild(net.wrap);
  box.appendChild(secN);

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
      base: draft.tier === 'vm' ? (draft.base.trim() || null) : null,
      pins: pins,
      net: draft.net,
    };
    save.disabled = true;
    c.setMsg('保存中…');
    try {
      await api('POST', '/api/sessions/' + encodeURIComponent(sid) + '/edit', body);
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
function cfgAgentBlock(a, idx, frozen, dupHints, syncDup) {
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
      syncDup();
    });
    const id = document.createElement('span'); id.textContent = m.id;
    const brief = document.createElement('span'); brief.className = 'wf-brief'; brief.textContent = m.brief || '';
    lab.appendChild(cb); lab.appendChild(id); lab.appendChild(brief);
    mods.appendChild(lab);
  }
  row.appendChild(field('模块（这个 agent 的能力；勾 1 个 = 直连式，勾多个 = 组合式）', mods));
  const dup = cfgHint('');
  dupHints.push(dup); // 与 draft.agents 同序：syncDup 按下标回写
  row.appendChild(dup);

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
    ? '已保存的是本机档：脚本用宿主自己的解释器，不装载运行包。下面这些是如实呈现的事实，缺包不会拦会话。'
    : '已保存的是虚拟机档：按模块声明的运行能力装载运行包；缺包 = 该模块的工具不执行（会话照样进行，如实说明缺什么）。'));

  const declared = rt.declared || {};
  const dIds = Object.keys(declared);
  sec.appendChild(cfgDiv('cfg-sub', '模块声明的运行能力'));
  if (!dIds.length) sec.appendChild(cfgDiv('cfg-line', '（没有模块声明运行能力：都不需要运行包）'));
  for (const id of dIds) sec.appendChild(cfgDiv('cfg-line', '· 模块 ' + id + ' 需要：' + (declared[id] || []).join('、')));

  const missing = rt.missing || {};
  const mIds = Object.keys(missing);
  sec.appendChild(cfgDiv('cfg-sub', '缺口（包库里没有的能力）'));
  if (!mIds.length) {
    sec.appendChild(cfgDiv('cfg-line cfg-ok', '没有缺口：模块声明的运行能力在依赖文件夹里都能找到。'));
  } else {
    for (const id of mIds) sec.appendChild(cfgDiv('cfg-line cfg-bad', '· 模块 ' + id + ' 缺：' + (missing[id] || []).join('、')));
    sec.appendChild(cfgHint('把对应的运行包放进 ' + (dir || '依赖文件夹 runtimes/') + '，再点上面的「重新扫描」（放入即出现，不用重启）。', 'warn'));
  }

  const avail = rt.available || {};
  const aCaps = Object.keys(avail);
  sec.appendChild(cfgDiv('cfg-sub', '包库里可用'));
  if (!aCaps.length) sec.appendChild(cfgDiv('cfg-line', '（依赖文件夹里没有可用的运行包）'));
  for (const cap of aCaps) sec.appendChild(cfgDiv('cfg-line', '· ' + cap + '：' + (avail[cap] || []).join('、')));

  const diags = rt.diagnoses || [];
  if (diags.length) {
    sec.appendChild(cfgDiv('cfg-sub', '虚拟机档诊断（选型不成立的原因）'));
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

/* 侧栏：只保留「被拒收模块」的如实提示（模块清单在「新建工作」向导里用）。 */
function renderSidebar() {
  const rejected = $('#rejected');
  if (rejected) rejected.textContent = (state.rejected || []).join('\n');
}

function toggleSettings() {
  state.settingsOpen = !state.settingsOpen;
  $('#settings-head').className = 'submenu-head' + (state.settingsOpen ? ' open' : '');
  $('#settings-body').className = 'submenu-body' + (state.settingsOpen ? ' open' : '');
}

/* ---------- 通用小构件（只用冒烟桩支持的 API） ---------- */
function btn(label, cls) {
  const b = document.createElement('button');
  b.type = 'button'; b.className = cls || 'btn'; b.textContent = label;
  return b;
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
  const close = () => { root.innerHTML = ""; root.className = "notice-root"; document.removeEventListener("keydown", onKey); };
  const onKey = (e) => { if (e.key === "Escape") close(); };
  ok.onclick = close;
  row.appendChild(ok);
  panel.appendChild(head); panel.appendChild(body); panel.appendChild(row);
  overlay.appendChild(panel);
  overlay.onclick = (e) => { if (e.target === overlay) close(); };
  document.addEventListener("keydown", onKey);
  root.appendChild(overlay);
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
    const done = (v) => { root.innerHTML = ""; root.className = "notice-root"; document.removeEventListener("keydown", onKey); resolve(v); };
    const onKey = (e) => { if (e.key === "Escape") done(false); };
    const no = btn("取消", "btn"); no.onclick = () => done(false);
    const yes = btn(confirmLabel || "确认", "btn btn-primary"); yes.onclick = () => done(true);
    row.appendChild(no); row.appendChild(yes);
    panel.appendChild(head); panel.appendChild(body); panel.appendChild(row);
    overlay.appendChild(panel);
    overlay.onclick = (e) => { if (e.target === overlay) done(false); };
    document.addEventListener("keydown", onKey);
    root.appendChild(overlay);
  });
}

/* ---------- 主区弹层（清空一律 innerHTML=''） ---------- */
function closeModal() {
  const root = $('#modal-root');
  root.innerHTML = '';
  root.className = 'modal-root';
}
function openModal(title, build, wide) {
  const root = $('#modal-root');
  root.innerHTML = '';
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
  overlay.onclick = (e) => { if (e.target === overlay) closeModal(); };
  root.appendChild(overlay);
  const ctx = {
    body,
    setMsg: (t, isErr) => { msg.textContent = t || ''; msg.className = isErr ? 'modal-msg err' : 'modal-msg'; },
    close: closeModal,
  };
  build(ctx);
  return ctx;
}

/* ---------- 设置①：供应商登记（录入 / 修改 / 删除） ---------- */
function openProvidersModal() {
  openModal('供应商登记', (c) => {
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
            await api('POST', '/api/providers/' + encodeURIComponent(p.id) + '/remove');
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
        await api('POST', '/api/providers', { id, base_url: url, api_key: key });
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
  });
}

/* ---------- 设置②：模型登记（录入 / 修改 / 删除 + 供应商模型发现） ---------- */
function openModelsModal() {
  openModal('模型登记', (c) => {
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
            await api('POST', '/api/models/' + encodeURIComponent(m.id) + '/remove');
            await refreshState(); rebuild();
            c.setMsg('已删除 ' + m.id);
          } catch (e) { c.setMsg(e.message, true); }
        };
        // 实测这条通道支不支持原生工具调用（要真实网络）：结论由后端如实回报，只写确定的结论。
        const probe = btn('测工具调用', 'link-btn');
        probe.onclick = async () => {
          probe.disabled = true;
          c.setMsg('正在实测 ' + m.id + ' 的工具调用支持（要发两条最小请求）…');
          try {
            const r = await api('POST', '/api/models/' + encodeURIComponent(m.id) + '/probe', {});
            const what = r.outcome === 'supported' ? '支持原生工具调用'
              : r.outcome === 'unsupported' ? '**不支持**原生工具调用（供应商拒了带 tools 的请求）'
              : '无法判定（供应商接受了 tools，但这次没有发起调用）';
            await refreshState(); rebuild();
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
            const r = await api('POST', '/api/models/' + encodeURIComponent(m.id) + '/probe-replay', {});
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
            const r = await api('POST', '/api/providers/' + encodeURIComponent(p.id) + '/discover', {});
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
        await api('POST', '/api/models', body);
        await refreshState(); rebuild();
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
  });
}

/* ---------- 设置③：核心 AI 默认模型 ---------- */
function openCoreModal() {
  openModal('核心 AI 默认模型', (c) => {
    const opts = state.models.map((m) => ({ value: m.id, label: m.name + '（' + m.id + '）' }));
    const sel = selectInput(opts, state.core);
    const save = btn('保存', 'btn btn-primary btn-block');
    save.onclick = async () => {
      const id = sel.value;
      if (!id) { c.setMsg('请先登记模型', true); return; }
      try {
        await api('POST', '/api/models/' + encodeURIComponent(id) + '/core');
        await refreshState();
        c.setMsg('核心 AI 默认模型：' + id);
      } catch (e) { c.setMsg(e.message, true); }
    };
    if (!opts.length) c.body.appendChild(emptyHint('（未登记模型，先到「模型登记」添加）'));
    c.body.appendChild(field('核心 AI 使用的模型', sel));
    c.body.appendChild(save);
  });
}

/* ---------- 设置④：基本设置 ---------- */
function openSettingsModal() {
  openModal('基本设置', (c) => {
    const stream = checkbox('流式传输（供应商逐片返回，边收边显示）', state.settings.streaming);
    const cot = checkbox('思维链显示（每条回答下的思维链，永远默认折叠、点击展开）', state.settings.show_reasoning);
    // 单次模型调用的总预算（全局：讨论 / 执行 / 验收 / 单 agent 共用）。
    const to = numberInput('单次模型调用的超时（秒）', state.settings.llm_timeout_secs, 10, 3600);
    // 讨论阶段的提醒次数（用户可调）：一轮内对同一个成员最多提醒几次。
    // **调用次数没有上限**：模型继续核实就继续跑，直到它给出表态（或用户点停止）。
    const remind = numberInput('一轮内最多提醒几次', state.settings.discuss_remind_cap, 0, 20);
    // 上下文用到多少就该压（占模型窗口的百分比）。
    const pct = numberInput('上下文用到百分之多少就压缩', state.settings.compact_at_percent, 0, 100);
    const save = btn('保存', 'btn btn-primary btn-block');
    save.onclick = async () => {
      try {
        await api('POST', '/api/settings', {
          streaming: stream.box.checked,
          show_reasoning: cot.box.checked,
          llm_timeout_secs: Number(to.input.value) || 300,
          discuss_remind_cap: Number(remind.input.value) || 0,
          compact_at_percent: Number(pct.input.value) || 0,
        });
        await refreshState();
        c.setMsg('已保存');
      } catch (e) { c.setMsg(e.message, true); }
    };
    c.body.appendChild(stream.wrap);
    c.body.appendChild(cot.wrap);
    c.body.appendChild(to.wrap);
    c.body.appendChild(cfgHint('超时是全局的：讨论、执行、验收与单 agent 共用这一份预算。用尽时会中断本轮并提示，点「继续」可重试（会话不会作废）。'));
    c.body.appendChild(remind.wrap);
    c.body.appendChild(cfgHint('调用次数没有上限：模型继续核实就继续跑，直到它给出表态（你可以随时点「停止」）。'));
    c.body.appendChild(cfgHint('一轮内提醒到顶就记一行「未回应」放过它，整轮继续（不阻塞）。'));
    c.body.appendChild(pct.wrap);
    c.body.appendChild(cfgHint('压缩由 AI 自己做：到点自动压一次；也可以随时手动 /compact。填 0 = 不自动压。'));
    c.body.appendChild(save);
  });
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
function openAgentsModal() {
  openModal('agent 管理', (c) => {
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
            await api('POST', '/api/agents/' + encodeURIComponent(a.name) + '/remove');
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
      if (!modules.length) { c.setMsg('至少勾选一个模块', true); return; }
      try {
        if (editing && editing !== name) {
          await api('POST', '/api/agents/' + encodeURIComponent(editing) + '/remove');
        }
        await api('POST', '/api/agents', { name, modules, model: modelSel.value || null, note: noteIn.value.trim() });
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
  }, true);
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
      await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/upload', { name: f.name, data_base64: b64 });
      filesCache.delete(s.sid); // 文件清单变了，@ 菜单下次重拉
      notice('已上传', '已上传到本次工作的 work/：' + f.name);
    } catch (err) {
      if (err.status === 409) conflictUpload(s.sid, f.name, b64);
      else notice('操作失败', err.message, 'err');
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
    const body = { name, data_base64: b64 };
    if (overwrite) body.overwrite = true;
    await api('POST', '/api/sessions/' + encodeURIComponent(sid) + '/upload', body);
    filesCache.delete(sid); // 文件清单变了，@ 菜单下次重拉
    notice('已上传', '已上传：' + name);
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
        await api('POST', '/api/sessions/' + encodeURIComponent(sid) + '/upload', { name: v, data_base64: b64 });
        filesCache.delete(sid); // 文件清单变了，@ 菜单下次重拉
        closeModal(); notice('已上传', '已上传：' + v);
      } catch (e) {
        if (e.status === 409) { c.setMsg('还是同名，请换一个名字', true); return; }
        c.setMsg(e.message, true);
      }
    };
    c.body.appendChild(field('文件名', inp));
    c.body.appendChild(go);
  });
}
/* ---------- 新建工作向导 ---------- */
function openWizard() {
  // single 形态：w.modules = 勾选的模块；w.agentPick = 复用的登记处 agent
  // （null = 用勾选的模块组临时 agent，名字见 w.agentName）。
  const w = { mode: 'single', modules: [], agentPick: null, agentName: '', agents: [], task: '' };
  const ed = { open: false, name: '', modules: [], model: null, note: '' };

  openModal('新建工作', (c) => {
    const nameIn = textInput('工作名称（必填，会话落盘目录名）');
    const modeSel = selectInput([
      { value: 'single', label: '单 agent（1 个或多个模块）' },
      { value: 'collab', label: '协作（多个 agent）' },
    ], 'single');
    // 形态预设标签保留：单 agent 里 1 个模块就是"直连式"，多个就是"组合式"。
    const modeHint = document.createElement('div'); modeHint.className = 'wf-hint';
    modeHint.textContent = '单 agent：选 1 个模块 = 直连式；选多个 = 组合式。';
    const partWrap = document.createElement('div'); partWrap.className = 'wf-field';
    const modelWrap = document.createElement('div'); modelWrap.className = 'wf-models';
    const editorWrap = document.createElement('div'); editorWrap.className = 'wf-field hidden';
    const taskIn = areaInput('本次需求（协作必填；也可写上，核心据此推荐）');
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
              await api('POST', '/api/agents', { name: a.name.trim(), modules: a.modules.slice(), model: a.model, note: '' });
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
        if (!ed.modules.length) { c.setMsg('至少勾选一个模块', true); return; }
        w.agents.push({ name: ed.name.trim(), transient: true, modules: ed.modules.slice(), model: ed.model, why: null });
        ed.open = false; renderAll(); c.setMsg('已加入临时 agent：' + ed.name.trim());
      };
      const addS = btn('保存为 agent 并加入', 'btn btn-primary');
      addS.onclick = async () => {
        if (!ed.name.trim()) { c.setMsg('agent 名字不能为空', true); return; }
        if (!ed.modules.length) { c.setMsg('至少勾选一个模块', true); return; }
        try {
          await api('POST', '/api/agents', { name: ed.name.trim(), modules: ed.modules.slice(), model: ed.model, note: ed.note.trim() });
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
      const lab = document.createElement('div'); lab.className = 'wf-label';
      lab.textContent = w.mode === 'single' ? '单 agent：复用已有 agent，或勾选它的模块'
        : 'agent（协作：可多个，各自独立沙箱）';
      partWrap.appendChild(lab);
      partWrap.appendChild(w.mode === 'single' ? singlePick() : agentPicker());
    }

    function renderModels() {
      modelWrap.innerHTML = '';
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

    function renderAll() { renderParts(); renderModels(); renderEditor(); }

    modeSel.addEventListener('change', () => {
      w.mode = modeSel.value;
      ed.open = false;
      renderAll();
    });

    recBtn.onclick = async () => {
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
          if (!mods.length) { c.setMsg('核心推荐的 agent 没有模块', true); return; }
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
    async function submit(body) {
      try { await startSession(body); closeModal(); } catch (e) { notice('操作失败', e.message, 'err'); }
    }

    createBtn.onclick = async () => {
      const name = nameIn.value.trim();
      if (!name) { c.setMsg('工作名称必填', true); return; }
      const task = taskIn.value.trim();
      let agents;
      if (w.mode === 'single') {
        if (w.agentPick) {
          const mods = (w.agentPick.modules || []).slice();
          if (!mods.length) { c.setMsg('单 agent：所选 agent 没有模块', true); return; }
          agents = [{
            name: w.agentPick.name, transient: false, modules: mods,
            model: w.model || w.agentPick.model || state.core,
          }];
        } else {
          const mods = pickedModules();
          if (!mods.length) { c.setMsg('单 agent：至少勾选 1 个模块，或在上方复用已有 agent', true); return; }
          agents = [{
            name: (w.agentName || '').trim() || mods[0], transient: true, modules: mods,
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
      const body = { name, mode: w.mode, agents };
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
    c.body.appendChild(partWrap);
    c.body.appendChild(modelWrap);
    c.body.appendChild(field('本次需求', taskIn));
    c.body.appendChild(recBtn);
    c.body.appendChild(createBtn);
    c.body.appendChild(cancelBtn);

    renderParts(); renderModels();
  }, true);
}

/* ---------- 会话 ---------- */
async function startSession(body) {
  const r = await api('POST', '/api/sessions', body);
  const sid = r.sid;
  const s = {
    sid, mode: body.mode, title: body.name || sid,
    lines: [], live: [], pending: null, busy: false, done: false, awaiting: null, fold: {}, scroll: {},
  };
  state.sessions.set(sid, s);
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
    // 任务链的进展：节点开工/回报/验收/交付——主会话也要看得到，不必点进子会话。
    case 'node_started':
      s.lines.push({ cls: 'sys system', who: '', text: '[节点] 开工：' + (ev.assignee || '') + ' · ' + (ev.node || '') });
      break;
    // 节点回报 = **它把活交回来了**：主会话记一行"完成"（摘要取首行，正文在子会话里看）。
    // 失败的节点由核心的验收结论另行如实说明（这里只显示"交回"这个事实，不替验收下判）。
    case 'report':
      s.lines.push({
        cls: 'sys system',
        who: '',
        text: '[节点] 完成：' + (ev.id || '') + (ev.rework ? '（第 ' + ev.rework + ' 轮返工）' : '')
          + (ev.text ? ' —— ' + String(ev.text).split('\n')[0].slice(0, 80) : ''),
      });
      break;
    case 'transcript':
      // 服务端权威转录：每行带会话内稳定 id（回档按 id 定位）。
      // 权威行到达：撤掉乐观回显与流式块，改用服务端的行；带 tool 的行渲染成工具卡片。
      s.lines = s.lines.filter((x) => !x.pending);
      for (const l of ev.lines) {
        // 系统消息（提醒、未回应这类不是谁说的内容）：独立样式，别和用户/发言混在一起。
        if (l.system) {
          s.lines.push({ cls: 'sys system', id: l.id, who: '', text: l.line });
          continue;
        }
        if (l.tool) {
          s.lines.push({ cls: 'tool', id: l.id, tool: l.tool, reasoning: l.reasoning || null, who: '', text: '', speaker: l.tool.speaker || '' });
          continue;
        }
        const parts = parseLine(l.line, l.degraded);
        for (const p of parts) { p.id = l.id; if (l.reasoning) p.reasoning = l.reasoning; }
        s.lines.push(...parts);
      }
      s.live = []; // 权威行整体替换掉流式块
      break;
    case 'delta': {
      // 流式块列表（短暂，不落盘）：一次发言可能有**多轮模型调用**（工具循环），
      // 所以按到达顺序排块——kind='start' 表示又一轮开始：封存上一块并推入新块，绝不丢弃。
      if (ev.kind === 'start' || !s.live.length || s.live[s.live.length - 1].kind !== 'msg') {
        s.live.push({ kind: 'msg', speaker: ev.speaker || '', segments: [] });
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
      s.working = ev.agent || null;
      s.busy = !!ev.agent;
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
    case 'decision':
      // 请用户裁决（短暂）：与快照里的 pending 是同一个事实，只是到达得更快。
      s.pending = ev;
      return false; // 门要整帧重画
    case 'ended': s.done = true; s.live = []; s.busy = false; break;
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
/* 正文看着就是"工具信封"（开头即 {"type":"tool"…）→ 兜底按工具卡片渲染。
 * 只在**开头**判定（允许前导空白），避免误伤正文中段引用 JSON 的情况。 */
function looksLikeToolEnvelope(text) {
  return /^\s*\{\s*"type"\s*:\s*"tool"/.test(String(text == null ? '' : text));
}

/* 以工具信封开头的文本行 → 卡片行对象（键用 L<行id>:raw，与真实工具调用的 T<序号> 分开）。 */
function envelopeLine(text, speaker) {
  return { cls: 'line', who: '', text: text, speaker: speaker || '', rawTool: true };
}

function parseLine(l, degraded) {
  const m = l.match(/^\[([^\]]+):([a-z]+)\]([\s\S]*)$/);
  if (m) {
    const cls = m[2] === 'agree' ? 'ok' : m[2] === 'leave' ? 'sys' : m[2] === 'ask' ? 'plan' : 'line';
    // 降级标记来自服务端的**结构化字段**（行上的 degraded），不靠匹配行文本里的说明文案。
    const deg = degraded === true;
    const text = m[3].trim();
    // 自由发言（say）的正文若整段就是工具信封，按卡片渲染，而不是当消息
    if (cls === 'line' && looksLikeToolEnvelope(text)) return [envelopeLine(text, m[1])];
    return [{ cls: deg ? 'sys' : cls, who: m[1] + ' · ' + m[2] + (deg ? ' · 信封缺失' : ''), text, speaker: m[1], verb: m[2], degraded: deg }];
  }
  if (l.startsWith('[用户')) return [{ cls: 'user', who: '用户', text: l.replace(/^\[[^\]]+\]\s*/, '') }];
  if (l.startsWith('[代拟]')) return [{ cls: 'sys', who: '核心代拟', text: l.slice(4) }];
  // 单 agent 的文本行：[说话人] 正文（协作的 [名字:动词] 上面已认）。
  // 正文**可能为空**：那一轮只思考、或只发了工具信封（思维链在 reasoning 里，正文没有）。
  // 核心自己的标签（[轮次 2] 这类）方括号里以「轮次」开头，保持系统行原样，不当成说话人。
  const sp = l.match(/^\[([^\]]+)\]\s*([\s\S]*)$/);
  if (sp) {
    const tag = sp[1];
    const text = sp[2].trim();
    // 核心自己的**分隔行**：轮次（讨论）与回合（agent 会话）都要有可见的分隔，
    // 否则一个 agent 的讨论段与执行段会糊成一片——"楼层丢失"就是这么来的（此前只认轮次）。
    // 核心自己的**分隔行**：轮次（讨论）与回合（agent 会话）都要有可见的分隔，
    // 否则一个 agent 的讨论段与执行段会糊成一片——"楼层丢失"就是这么来的（此前只认轮次）。
    if (!text && /^(轮次|回合)/.test(tag)) return [{ cls: 'sys system', who: '', text: l }];
    if (looksLikeToolEnvelope(text)) return [envelopeLine(text, tag)];
    return [{ cls: 'line', who: tag, text }];
  }
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
  // 忙碌时只留一个「停止」：用户一眼就知道这个会话在跑，而不是拿「继续/发送」去试探。
  const busy = isBusy(s);
  sendBtn.textContent = busy ? '停止' : '发送';
  sendBtn.className = busy ? 'btn btn-danger' : 'btn btn-primary';
  const cont = $('#btn-continue');
  if (cont) cont.className = busy ? 'btn hidden' : 'btn';
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

/// 回档按钮：删掉这一行和它之后的所有消息。
function rewindButton(id) {
  const b = document.createElement('button');
  b.className = 'line-act danger'; b.textContent = '删除此行和之后所有消息';
  b.title = '删除此行和之后所有消息';
  b.onclick = () => rewindTo(id);
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

/// 定稿行渲染到容器里（每次全量重建这个容器；折叠与 <pre> 滚动状态由 store 恢复）。
function renderDone(s) {
  if (!doneBox) return;
  doneBox.innerHTML = '';
  // tool 行的稳定序号：在 s.lines 里按出现顺序数（兜底的信封行不算，它用 L<行id>:raw）。
  let toolSeq = 0;
  for (const l of s.lines) {
    // 工具行（含"正文就是工具信封"的兜底行）：渲染成卡片，而不是当消息发出来。
    if (l.tool || l.rawTool) {
      const card = l.tool ? toolCard(l.tool, s, 'T' + toolSeq) : rawToolCard(l.text, s, 'L' + l.id, l.speaker);
      if (l.tool) toolSeq += 1;
      if (l.reasoning && state.settings.show_reasoning) card.appendChild(reasoningBlock(l.reasoning, s, 'L' + l.id));
      if (typeof l.id === 'number') card.appendChild(rewindButton(l.id));
      doneBox.appendChild(card);
      continue;
    }
    const el = document.createElement('div');
    el.className = 'line ' + l.cls;
    if (l.who) {
      const w = document.createElement('span'); w.className = 'who'; w.textContent = l.who; el.appendChild(w);
    }
    // 思维链在回答之上（先想后说）；默认折叠，点开状态会被记住（键 = L<行 id>）。
    if (l.reasoning && state.settings.show_reasoning) el.appendChild(reasoningBlock(l.reasoning, s, 'L' + l.id));
    appendBody(el, l.cls, l.text);
    // 删除：删掉这一行和它之后的所有消息（服务端按行 id 重建，前端整体替换）。
    if (typeof l.id === 'number') el.appendChild(rewindButton(l.id));
    // 撤回该同意：转录追加一条撤回行，继续时按剩余转录重新判定。
    if (l.verb === 'agree' && l.speaker) {
      const w = document.createElement('button');
      w.className = 'line-act'; w.textContent = '撤回同意'; w.title = '让该 agent 本轮不再算同意';
      w.onclick = () => withdrawAgree(l.speaker);
      el.appendChild(w);
    }
    doneBox.appendChild(el);
  }
  flushScroll(s);
}

/// 流式块：只做增量——块不存在就建节点，内容只在既有节点上更新，绝不重建。
/// **一个块最多一个思维链折叠 + 一段正文**（与定稿后的行同形）：模型会把"思维链/正文"交替吐出来，
/// 若按到达片段各建节点，流式期间就会冒出两个「思维链」、正文被切成好几段。
/// 流式 tool 块的键 = 已有权威 tool 行数 + 它在 live 里的 tool 块序号 → 与定稿后的权威行同键。
function renderLive(s, full) {
  if (!liveBox) { renderStream(true); return; }
  const live = s.live || [];
  if (full) { for (const b of live) { b._node = null; b._cot = null; b._txt = null; } }
  const toolLines = s.lines.filter((l) => l.tool).length;
  let liveTool = 0;
  for (let bi = 0; bi < live.length; bi++) {
    const blk = live[bi];
    if (blk.kind === 'tool') {
      if (!blk._node) {
        blk._node = toolCard(blk.tool, s, 'T' + (toolLines + liveTool));
        liveBox.appendChild(blk._node);
      }
      liveTool += 1;
      continue;
    }
    const segs = blk.segments || [];
    if (!blk._node) {
      const el = document.createElement('div');
      el.className = 'line line streaming';
      const w = document.createElement('span'); w.className = 'who'; w.textContent = blk.speaker;
      el.appendChild(w);
      blk._node = el;
      blk._cot = null;
      blk._txt = null;
      liveBox.appendChild(el);
    }
    // 先把这一轮的所有片段按 kind 归并（顺序无关紧要：思维链永远在正文之上，与定稿行一致）
    let cot = '';
    let txt = '';
    for (const seg of segs) {
      const t = String(seg.text == null ? '' : seg.text);
      if (!t) continue;
      if (seg.kind === 'reasoning') cot += t;
      else txt += t;
    }
    const showCot = !!state.settings.show_reasoning;
    if (showCot && cot) {
      if (!blk._cot) {
        const node = reasoningBlock(cot, s, 'V' + bi);
        // 正文节点若先到，思维链要插到它前面（永远"先想后说"）
        if (blk._txt && blk._txt.node && typeof blk._node.insertBefore === 'function') {
          blk._node.insertBefore(node, blk._txt.node);
        } else {
          blk._node.appendChild(node);
        }
        blk._cot = { node: node, text: cot };
      } else if (blk._cot.text !== cot) {
        if (blk._cot.node && blk._cot.node.children && blk._cot.node.children[1]) {
          blk._cot.node.children[1].textContent = cot;
        }
        blk._cot.text = cot;
      }
    }
    if (txt) {
      if (!blk._txt) {
        const node = mdNode(txt);
        blk._node.appendChild(node);
        blk._txt = { node: node, text: txt };
      } else if (blk._txt.text !== txt) {
        // 只更新内容，节点不动 → 折叠与滚动都不受打扰
        if (typeof markdownToHtml === 'function') blk._txt.node.innerHTML = shortPath(markdownToHtml(txt));
        else blk._txt.node.textContent = txt;
        blk._txt.text = txt;
      }
    }
    // 这一轮没有**可见**内容（例如只发了一封信封、正文不外泄）→ 整块隐藏：
    // 别留一张只有说话人名字的空卡片（工具轮尤其明显）。节点位置不变，所以内容后到时会出现在正确位置。
    const visible = (showCot && !!cot) || !!txt;
    if (visible) blk._node.classList.remove('empty');
    else blk._node.classList.add('empty');
  }
  flushScroll(s);
}

/// 整帧重建：容器与全部行都重建（折叠 / <pre> 滚动由 store 恢复）。
function renderStream(force) {
  const box = $('#stream');
  const prevTop = box.scrollTop;
  const stick = force === true || nearBottom(box);
  // 容器随之重建，避免旧节点被当成"已经渲染过"
  doneBox = null; liveBox = null; typingNode = null; domOwner = null;
  box.innerHTML = '';
  const s = activeSession();
  if (!s) {
    box.innerHTML = '<div class="empty"><div class="logo">☉</div>' +
      '<div class="tip">点左上角「+ 新建工作」开始：单 agent（直连式 / 组合式）· 协作</div>' +
      '<div class="hint">PC：双栏布局　移动端：左上角 ☰ 打开侧栏</div></div>';
    return;
  }
  buildStreamBoxes(s.sid);
  renderDone(s);
  renderLive(s, true);
  syncTyping(s);
  box.scrollTop = stick ? box.scrollHeight : prevTop;
  syncSendButton(s);
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

/* 裁决门：核心请用户定的事。二选一的（名单/开始）给按钮；其余给**自由文本**。
   改需求**不是**门——它是会话级按钮（见 syncSendButton / updateTaskFlow）。 */
function renderGate(s) {
  const gate = $('#gate');
  gate.innerHTML = '';
  if (!s || isBusy(s) || s.done || s.readonly) return;
  if (s.awaiting === 'task') {
    gate.appendChild(gateCard('请提交本次协作需求：', [
      ['提交', async () => { const v = takeInput(); if (v) await act('task', v); }],
    ]));
    return;
  }
  const p = s.pending;
  if (!p) return;
  if (p.kind === 'confirm_slate') {
    gate.appendChild(gateCard('核心已代拟名单（见转录），是否按此建组？', [
      ['确认建组', () => act('slate', 'yes')],
      ['取消', () => act('slate', 'no')],
    ]));
  } else if (p.kind === 'confirm_begin') {
    gate.appendChild(gateCard('名单已定，开始讨论？', [
      ['开始', () => act('begin', 'yes')],
      ['开始（授权小组自裁细节）', () => act('begin', 'yes,allow')],
      ['暂不', () => {}],
    ]));
  } else {
    gate.appendChild(decisionCard(p));
  }
}

/// 裁决卡：**核心的说明 + 建议 + 要你回答的那句 + 自由文本**。
/// 用户写自己的想法即可（"马上做"这种自然语言就算明确）；核心 AI 判定意图是否明确，明确了才开工/放行。
function decisionCard(p) {
  const el = document.createElement('div');
  el.className = 'gate-card';
  const q = document.createElement('div'); q.className = 'q'; q.textContent = p.summary || ''; el.appendChild(q);
  if (p.advice) {
    const a = document.createElement('div'); a.className = 'advice'; a.textContent = '建议：' + p.advice; el.appendChild(a);
  }
  if (p.question) {
    const qq = document.createElement('div'); qq.className = 'ask'; qq.textContent = p.question; el.appendChild(qq);
  }
  const hint = document.createElement('div');
  hint.className = 'hint';
  hint.textContent = '用你自己的话说一句——它会进主会话，所有成员都看得到。';
  el.appendChild(hint);
  const inp = document.createElement('input');
  inp.className = 'decision-input';
  inp.placeholder = '你的想法…';
  el.appendChild(inp);
  const bs = document.createElement('div'); bs.className = 'btns';
  const b = document.createElement('button'); b.className = 'btn btn-primary'; b.textContent = '提交';
  b.onclick = async () => { const v = inp.value && inp.value.trim(); if (v) await act('decide', v); };
  bs.appendChild(b); el.appendChild(bs);
  return el;
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
    const r = await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/update-task', { text });
    s.lines = []; s.pending = null; s.readonly = false; s.done = false;
    for (const ev of (r.events || [])) absorb(s, ev);
    renderAll();
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
    for (const ev of (r.events || [])) absorb(s, ev);
    renderStream(true);
  } catch { /* 拿不到就等下一次状态刷新 */ }
}

/** 收一批事件并应用。返回 'full' | 'live' | 'none'（渲染粒度）。 */
function applyBatch(seq, sid, events) {
  if (typeof seq === 'number') appliedSeq = Math.max(appliedSeq, seq);
  let s = state.sessions.get(sid);
  if (!s) {
    // 未知会话 = 有**别的客户端**建了会话（演示脚本、另一个标签页）。
    // **必须就地建出会话状态**再吸收事件：否则它的事件（含流式 delta）会被永久丢掉，
    // 只有手动点开时才靠历史回放补上——那正是"流式没起效、别人的会话不更新"的来源。
    const h = (state.history || []).find((x) => x.name === sid);
    const v = (state.views || new Map()).get(sid);
    s = {
      sid, mode: (h && h.mode) || 'single', title: sid,
      lines: [], live: [], pending: (v && v.pending) || null, busy: false,
      can_update_task: !!(v && v.can_update_task),
      done: !!(h && h.done), awaiting: null, fold: {}, scroll: {},
    };
    state.sessions.set(sid, s);
    needState = true; // 侧栏也顺手对齐
  }
  // 运行态：别人在跑时前端自己的 busy 不知道，用事件流推断（有增量=在跑；见到收尾=跑完）。
  if (events.some((e) => e.type === 'delta')) s.busy = true;
  if (events.some((e) => e.type === 'ended' || e.type === 'delivery' || e.type === 'discussion_done')) s.busy = false;
  return absorbEvents(s, events);
}
async function act(action, text) {
  const s = activeSession();
  if (!s || isBusy(s) || s.readonly) return;
  s.busy = true;
  // 乐观回显：自己的发言立刻可见；服务端权威行到达时自动替换（见 absorb）。
  if (text && (action === 'say' || action === 'task' || action === 'answer')) {
    s.lines.push({ cls: 'user', who: '用户', text, pending: true });
  }
  renderStream();
  try {
    // 命令回包只有头部序号：事实（含自己那条发言的权威行）由事件流补进来。
    await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/' + action, { text });
    s.awaiting = null;
    renderAll();
  } catch (err) {
    s.lines = s.lines.filter((x) => !x.pending);
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    renderAll();
  } finally {
    s.busy = false;
    renderAll();
  }
}

/* 删除：删掉这一行和它之后的所有消息；服务端返回重放后的完整事件流，前端整体重建。 */
function rewindTo(id) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  choiceModal('删除消息', '删除这一行和之后的所有消息？此操作不可撤销。', [
    ['删除', 'btn btn-danger', async () => {
      try {
        const r = await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/rewind', { id });
        s.lines = [];
        s.live = [];
        s.fold = {};   // 行整体重建，折叠状态一并重来（避免旧键被新行复用）
        s.scroll = {};
        s.pending = null;
        s.done = false;
        s.readonly = false; // 历史回放会话一旦删除即转为活动会话
        for (const ev of (r.events || [])) absorb(s, ev);
        renderAll();
      } catch (err) { notice('操作失败', err.message, 'err'); }
    }],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 撤回某 agent 的同意（转录追加撤回行，协作才有意义）；值 = agent 实例名。 */
function withdrawAgree(agent) {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  choiceModal('撤回同意', '撤回「' + agent + '」的同意？继续时会按剩余转录重新判定。', [
    ['撤回', 'btn btn-danger', async () => {
      try {
        await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/withdraw', { agent });
        renderAll();
      } catch (err) { notice('操作失败', err.message, 'err'); }
    }],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 继续：由用户点击授权核心往下走。单 agent 若末条是 AI，服务端只回提醒、不发请求。 */
async function continueFlow() {
  const s = activeSession();
  if (!s || isBusy(s)) return;
  s.busy = true;
  renderStream();
  try {
    await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/continue', {});
    s.readonly = false; // 历史回放会话一旦继续即转为活动会话（跨重启续跑）
    renderAll();
  } catch (err) {
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    renderAll();
  } finally {
    s.busy = false;
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
  // busy（生成中）仍拒绝，避免与「停止」抢键盘；readonly（历史只读会话）允许——插入文本无害。
  if (!tok || !s || isBusy(s)) { atClose(); return; }
  // 同步先开菜单：/files 还没有回来，↑↓/Enter 也必须已经被菜单接管，
  // 否则这期间按 Enter 会把消息直接发出去。
  atState.open = true;
  atState.loading = true;
  atState.items = []; atState.all = []; atState.active = 0;
  atState.sid = s.sid; atState.start = tok.start;
  atRender();
  const data = await atLoad(s.sid);
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
$('#btn-continue').onclick = continueFlow;
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
    await api('POST', '/api/sessions/' + encodeURIComponent(s.sid) + '/stop', {});
  } catch (e) { /* 停止失败不吵用户；按钮仍是停止，可再点一次 */ }
}

function onSend() {
  const s = activeSession();
  if (!s || s.readonly) return;
  if (isBusy(s)) { stopGeneration(); return; } // 生成中：同一个键变成「停止」
  if (s.awaiting === 'task') { const v = takeInput(); if (v) act('task', v); return; }
  // 裁决是自由文本：把输入框里的话作为回应提交（核心 AI 判定意图是否明确）。
  if (s.pending && s.pending.kind !== 'confirm_slate' && s.pending.kind !== 'confirm_begin') {
    const v = takeInput();
    if (v) act('decide', v);
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
          // 不在这里改 busy：流式增量到达时会把「停止」按钮误翻回「发送」。
        } catch { /* 单行损坏不拖垮轮询 */ }
      }
      // 状态对齐：别的客户端（演示脚本、另一个标签页）建/删/改会话时，侧栏自己跟上——
      // 不需要用户刷新浏览器；未知会话会立刻置 needState，其余靠 3 秒兜底。
      if (needState || Date.now() - lastStateAt > 3000) {
        needState = false;
        lastStateAt = Date.now();
        // refreshState 自己会把侧栏与历史重画；agent 登记弹窗里的列表归那个弹窗自己管。
        // refreshState 自己会把侧栏与历史重画；agent 登记弹窗里的列表归那个弹窗自己管。
        // 曾经这里调了弹窗内部的 `renderList`——那个名字在顶层作用域根本不存在，
        // 于是每次刷新都抛一次 ReferenceError，被轮询的 catch 当成"断线"，状态点一直红着。
        await refreshState();
      }
      // 纯流式增量：只 append 新节点（折叠、<pre> 滚动、外层滚动都不被打断）。
      if (mode === 'full') renderAll();
      else if (mode === 'live') renderLiveTick(activeSession());
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

/* ---------- 抽屉（移动端） ---------- */
$('#btn-drawer').onclick = () => { $('#sidebar').classList.add('open'); $('#drawer-mask').classList.add('show'); };
$('#drawer-mask').onclick = () => { $('#sidebar').classList.remove('open'); $('#drawer-mask').classList.remove('show'); };

/* ---------- 顶层接线 ---------- */
$('#btn-new-work').onclick = openWizard;
$('#settings-head').onclick = toggleSettings;
$('#btn-providers').onclick = openProvidersModal;
$('#btn-models').onclick = openModelsModal;
$('#btn-core').onclick = openCoreModal;
$('#btn-settings').onclick = openSettingsModal;
$('#btn-agents').onclick = openAgentsModal;
$('#btn-upload').onclick = pickUploadFile;

/* ---------- 启动 ---------- */
refreshState().then(pollLoop).catch((err) => notice('初始化失败', err.message, 'err'));
