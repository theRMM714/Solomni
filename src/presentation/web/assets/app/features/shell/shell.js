/* 目的：应用外壳：拉 /api/state 并重画、登记处变化的广播、围栏/工具链横幅、长轮询的接线与连接状态。
 * 管：一次状态刷新要重画哪些地方、登记处指纹没变就不重画、事件流三条回调（裁剪对齐 / 批次应用 / 出错）怎么接、启动接线。
 * 不管：各功能怎么画（只调 app.* 上的入口）；事件吸收与渲染本身（见 features/transcript/）。
 * 联动：由 app/main.js 最后装配，并在所有功能挂好后调用 start()。 */

let doc, state, stores, api, ui, app;
let registry;
let eventStream;
const loadWork = (...a) => app.loadWork(...a);
const $ = (sel) => doc.querySelector(sel);

/** 模型下拉的选项：needEmpty = 带「（核心默认）」空项（会话配置用）。 */
function modelOptions(current, needEmpty) {
  const opts = (needEmpty ? [{ value: "", label: "（核心默认）" }] : [])
    .concat(state.models.map((m) => ({ value: m.id, label: m.name + "（" + m.id + "）" })));
  if (current && !state.models.some((m) => m.id === current)) {
    opts.push({ value: current, label: current + "（登记处已无此模型）" });
  }
  return opts;
}

function onRegistryChange(fn) { return registry.on(fn); }

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

/* 传输（取批次、游标、断线重试）在 app/core/events.js；这里只管裁剪对齐与批次的应用。
 * 取事件失败才算断线（setConn）；应用 / 渲染出错走 eventError，连接照常是好的。 */


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
  try { ui.notice('界面处理事件出错', msg, 'err'); } catch { /* 弹窗本身出问题就只剩控制台 */ }
}

export function mount(root, deps) {
  ({ doc, state, stores, api, ui, app } = deps);
  // 登记处一变，所有已打开的分页都跟着同步列表与下拉选项（stores.createRegistryWatch）。
  registry = stores.createRegistryWatch(state, (e) => console.error("登记处视图刷新失败：", e));
  // 传输（取批次、游标、断线重试）在 core/events.js；这里只接三条回调。
  eventStream = deps.createEventStream({
    onConn: setConn,
    onTrim: (...a) => app.trimAlign(...a),
    onLines: (...a) => app.applyLines(...a),
    onError: eventError,
  });
  Object.assign(root, { modelOptions, registry, onRegistryChange, refreshState, renderFenceBanner, renderToolchainBanner, eventStream, pollLoop, setConn, eventError });
}

/* 启动接线：输入区、抽屉遮罩、首次刷新；由 main.js 在所有功能挂好之后调用。 */
export function start() {
  $('#btn-send').onclick = (...a) => app.onSend(...a);
  $('#btn-continue').onclick = (...a) => app.continueContext(...a);
  $('#btn-update-task').onclick = (...a) => app.updateTaskFlow(...a);
  $('#input').addEventListener('keydown', (e) => {
    if (app.atKey(e)) return; // 菜单打开时 ↑/↓/Enter/Esc 归菜单
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); app.onSend(); }
  });
  $('#input').addEventListener('input', () => { app.autoGrow(); app.atOnInput(); });

  $('#drawer-mask').onclick = () => app.closeDrawer();
  $('#btn-new-work').onclick = () => app.openWizard();
  $('#btn-settings').onclick = () => app.openSettingsHub('generate');
  $('#fence-settings').onclick = () => app.openSettingsHub('sandbox');
  $('#btn-upload').onclick = () => app.pickUploadFile();
  app.setupSidebar();

  refreshState()
    .then(pollLoop)
    .catch((err) => ui.notice('初始化失败', err.message, 'err'));
}
