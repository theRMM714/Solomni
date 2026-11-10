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
const { btn } = ui;

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
  for (const cur of opened) app.hydrateHistory(cur);
  // 登记处真变了才通知：轮询每 3 秒都会进来，每次重画会让用户填到一半的表单被冲掉。
  const registryChanged = registry.check();
  renderFenceBanner();
  renderToolchainBanner();
  app.renderSidebar();
  app.renderHistory();
  if (opened.length) app.renderTabs();
  const cur = app.activeSession();
  if (cur) {
    app.syncTyping(cur); app.syncSendButton(cur); app.syncComposer(cur);
    // 提交信息不塞进每 3 秒的 /api/state：按需、低频刷新（后台 agent 提交也能跟上）。
    if (!cur.workAt || Date.now() - cur.workAt > 15000) loadWork(cur.sid);
  }
  if (registryChanged) registry.notify();
}

/* ---------- 配置视图（会话列表的「编辑」）：名单 / 模块 / 模型 / 档位 / 运行能力 / 定版 ----------
 * 读：GET /api/sessions/{sid}/config；写：POST /api/sessions/{sid}/edit。
 * 「重新扫描」= 重新 GET 一次并重绘：模块清单与依赖文件夹在每次读取时重扫，不需要额外接口。
 * 冻结口径与后端一致：started=true 时只冻结 agent 名单与形态（名字只读、不能增删 agent），模块与模型仍可改。
 * 保存失败一律把后端 error 原文显示出来，不在前端兜底改写。 */


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

/* ---------- 渲染 ---------- */

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
    if (state.activeSid === sid) app.syncStatusbar(s);
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
    : app.fmtDuration(shellSecs(sh)));
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
    o.dur.textContent = app.fmtDuration(shellSecs(sh));
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
  const pct = app.ctxPercent(c);
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
  box.appendChild(popoverRow('已用', app.fmtInt(c.total_tokens) + ' tokens'));
  box.appendChild(popoverRow('窗口', app.fmtInt(c.window) + ' tokens'));
  box.appendChild(popoverRow('占用', pct == null ? '—' : pct.toFixed(1) + '%'));
  box.appendChild(popoverRow('上次输入 / 输出', app.fmtInt(c.prompt_tokens) + ' / ' + app.fmtInt(c.completion_tokens)));
  if (c.cache_read != null || c.cache_creation != null) {
    box.appendChild(popoverRow(
      '缓存读 / 写',
      (c.cache_read == null ? '—' : app.fmtInt(c.cache_read)) + ' / ' +
        (c.cache_creation == null ? '—' : app.fmtInt(c.cache_creation)),
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
    : (app.isBusy(s) ? '生成中，先停止或等这一轮结束' : (s.readonly ? '历史回放为只读' : ''));
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
  if (app.isBusy(s)) { ui.toast('warn', '会话正在生成：先停止再切换'); return; }
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
    box.appendChild(popoverRow('作者 / 时间', h.author + ' · ' + app.timeAgo(h.time)));
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
        meta.textContent = (c.on_head ? '当前线 · ' : '已离开当前线 · ') + c.author + ' · ' + app.timeAgo(c.time);
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
      detail.appendChild(popoverRow('作者 / 时间', d.author + ' · ' + app.fmtDateTime(d.time)));
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
      const blocked = s.readonly ? '历史回放只读：先启用会话' : (app.isBusy(s) ? '会话正在生成：先停止再回滚' : '');
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
        meta.textContent = (cm.on_head ? '当前线 · ' : '已离开 · ') + cm.author + ' · ' + app.timeAgo(cm.time) + ' · ' + cm.changes + ' 处';
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
        app.syncStatusbar(s);
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
  if (app.isBusy(s)) { notice('会话正在生成', '先停止再回滚，避免与正在提交的 agent 交错。', 'info'); return; }
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

/* ---------- 动作 ---------- */

/* 本地名 → 动作 id 的映射：动作目录与校验都在核心（systools/tools.yaml），这里只做转写。 */
const ACT_IDS = { say: 'send_message', task: 'set_task' };
/* 动作只有一个入口 /api/actions/{id}：声明、授权、执行与审计都在核心那一处。 */
function actionUrl(id) { return '/api/actions/' + id; }
async function act(action, text) {
  const s = app.activeSession();
  if (!s) return;
  if (app.isBusy(s)) return;
  if (s.readonly) { notice('历史回放为只读', '点「启用会话」后才能发送消息。', 'info'); return; }
  // 本地事实：我的命令在途（不是服务端运行态）——按钮立刻切「停止」，避免重复提交。
  s.sending = true;
  // 乐观回显：自己的发言立刻可见；服务端权威行到达时自动替换（见 app.absorb）。
  if (text && (action === 'say' || action === 'task' || action === 'answer')) {
    s.lines.push({ cls: 'user', who: '用户', text, pending: true });
  }
  app.renderStream();
  try {
    // 命令回包只有头部序号：事实（含自己那条发言的权威行）由事件流补进来。
    await api('POST', actionUrl(ACT_IDS[action] || action), { session_id: s.sid, text });
    s.awaiting = null;
    app.renderAll();
  } catch (err) {
    s.lines = s.lines.filter((x) => !x.pending);
    s.forceFull = true;
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    app.renderAll();
  } finally {
    s.sending = false;
    // 回包只给头部序号；事实由事件流补进来。顺手要一次快照**对账**：命令已结束但它的
    // 收尾事件可能还在路上，运行态由事件（或这轮快照）说了算，不由"我在不在途"说了算。
    app.markNeedState();
    app.renderAll();
  }
}

/* 回档：留档（默认，可恢复）或删除（真的截断）；服务端回重放后的完整事件流，前端整体重建。 */
function rewindTo(id) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
  app.choiceModal('回档到此处', '留档：保留内容、折叠成标记，可随时恢复，工作区文件（共享区与各 agent 沙箱）也一起记入档案；删除：真的删掉这一行及其后的全部，工作区文件一起回到该行（未提交内容进 lost 备份），删除点之后的留档一并作废，不可恢复。', [
    ['留档', 'btn', () => applyRewind(s, { id, mode: 'archive' })],
    ['删除', 'btn btn-danger', () => applyRewind(s, { id, mode: 'delete' })],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 恢复某次留档：删掉该标记及其后的内容（含留档期间的新工作）。 */
function restoreMark(mark) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
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
    app.applySnapshot(s, (r.events || []).concat(r.live || []));
    s.workDirty = true;
    app.renderAll();
    ui.toast('ok', '已回档');
  } catch (err) {
    if (/被外部改动/.test(err.message) &&
        await confirmBox('覆盖工作区外部改动', err.message + '\n\n确认后会先采纳这些外部改动，再把工作区回档到目标提交；未跟踪文件不受影响。', '仍要回档')) {
      try {
        const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid, force: true }, body));
        if (typeof r.head === 'number') s.floor = r.head;
        app.applySnapshot(s, (r.events || []).concat(r.live || []));
        s.workDirty = true;
        app.renderAll();
        ui.toast('ok', '已确认覆盖并回档');
      } catch (e2) { notice('操作失败', e2.message, 'err'); }
      return;
    }
    if (/个留档/.test(err.message) &&
        await confirmBox('删除会作废留档', err.message + '\n\n确认后会连同共享区与各 agent 沙箱一起回到该行；这些留档作废后不可恢复。', '仍要删除')) {
      try {
        const r = await api('POST', actionUrl('rewind'), Object.assign({ session_id: s.sid, force: true }, body));
        if (typeof r.head === 'number') s.floor = r.head;
        app.applySnapshot(s, (r.events || []).concat(r.live || []));
        s.workDirty = true;
        app.renderAll();
        ui.toast('ok', '已确认删除');
      } catch (e2) { notice('操作失败', e2.message, 'err'); }
      return;
    }
    notice('操作失败', err.message, 'err');
  }
}

/* 撤回某 agent 的同意（转录追加撤回行，协作才有意义）；值 = agent 实例名。 */
function withdrawAgree(agent) {
  const s = app.activeSession();
  if (!s || app.isBusy(s)) return;
  app.choiceModal('撤回同意', '撤回「' + agent + '」的同意？继续时会按剩余转录重新判定。', [
    ['撤回', 'btn btn-danger', async () => {
      try {
        await api('POST', actionUrl('withdraw'), { session_id: s.sid, agent });
        app.renderAll();
        ui.toast('ok', '已撤回「' + agent + '」的同意');
      } catch (err) { notice('操作失败', err.message, 'err'); }
    }],
    ['取消', 'btn btn-ghost', () => {}],
  ]);
}

/* 启用：由用户点击授权核心恢复一条历史会话。后端负责重建 / 共享 guest / 解冻整棵树；
   前端只把返回的会话视图贴回本地并渲染。启用本身不跑模型。 */
async function activateFlow() {
  const s = app.activeSession();
  if (s && s.workspace_missing) { notice('工作区已不存在', '这条会话绑定的工作区已不可达：恢复目录或删除会话后再启用。', 'err'); return; }
  if (!s || !s.readonly || s.activating) return;
  s.activating = true;
  app.renderAll();
  try {
    const r = await api('POST', actionUrl('activate_session'), { session_id: s.sid });
    applyActivation(s, (r && r.session) || {});
    s.readonly = false;
    app.renderAll();
    ui.toast('ok', '会话已启用');
  } catch (err) {
    notice('启用失败', err.message, 'err');
  } finally {
    s.activating = false;
    app.renderAll();
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
  const s = app.activeSession();
  if (!s || s.readonly || app.isBusy(s) || !s.continue_action) return;
  const hint = s.continue_action;
  s.sending = true;
  s.continue_action = null; // 先收起；以事件与快照为准，失败时状态刷新会再给
  app.renderAll();
  try {
    await api('POST', actionUrl('control_session'), {
      session_id: s.sid,
      action: 'continue',
      reason: hint.reason || hint.label || '',
    });
  } catch (err) {
    s.lines.push({ cls: 'bad', who: '错误', text: err.message });
    app.renderAll();
  } finally {
    s.sending = false;
    app.markNeedState(); // 运行态以事件为准，快照只对账
    app.renderAll();
  }
}

/* ---------- 长轮询：增量事件 + 连接状态 ---------- */
/* 传输（取批次、游标、断线重试）在 app/core/events.js；这里只管裁剪对齐与批次的应用。
 * 取事件失败才算断线（setConn）；应用 / 渲染出错走 eventError，连接照常是好的。 */
const eventStream = core.createEventStream({
  onConn: setConn,
  onTrim: app.trimAlign,
  onLines: app.applyLines,
  onError: eventError,
});
function pollLoop() { return eventStream.run(); }

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

/* ---------- 输入区接线 ---------- */
$('#btn-send').onclick = app.onSend;
$('#btn-continue').onclick = app.continueContext;
$('#btn-update-task').onclick = app.updateTaskFlow;
$('#input').addEventListener('keydown', (e) => {
  if (app.atKey(e)) return; // 菜单打开时 ↑/↓/Enter/Esc 归菜单
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); app.onSend(); }
});
$('#input').addEventListener('input', () => { app.autoGrow(); app.atOnInput(); });

/* ---------- 抽屉遮罩 / 顶层接线 ---------- */
$('#drawer-mask').onclick = app.closeDrawer;
$('#btn-new-work').onclick = app.openWizard;
$('#btn-settings').onclick = () => app.openSettingsHub('generate');
$('#fence-settings').onclick = () => app.openSettingsHub('sandbox');
$('#btn-upload').onclick = () => app.pickUploadFile();
app.setupSidebar();

/* ---------- 启动 ---------- */
refreshState()
  .then(pollLoop)
  .catch((err) => notice('初始化失败', err.message, 'err'));
