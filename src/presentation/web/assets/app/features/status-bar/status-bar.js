/* 目的：顶部状态栏——上下文用量、思考档、权限档与共享区卡片入口。
 * 管：syncStatusbar / openPermissionMenu / openThinkingMenu / switchThinking / switchPermission。
 * 不管：不改会话内容；档位切换走统一动作分发。
 * 联动：档位目录来自 app/stores/settings.js；工作卡来自 app 上的 loadWork / openShellCard。 */

import { PERMISSION_LABELS, PERMISSION_ORDER, thinkingLabel } from '../../stores/settings.js';
import { ctxChipClass, shellAbnormal, shellRunning, shellSecs, statusPrintOf } from './print.js';
import { ctxPercent, fmtDuration, fmtInt, timeAgo } from '../../core/format.js';

let g, doc, state, api, ui, app;
let icon;
const actionUrl = (...a) => app.actionUrl(...a);
const openConfig = (...a) => app.openConfig(...a);
const openContextCard = (...a) => app.openContextCard(...a);
const openShellCard = (...a) => app.openShellCard(...a);
const openToolsMenu = (...a) => app.openToolsMenu(...a);
const openWorkCard = (...a) => app.openWorkCard(...a);
const paintShellCard = (...a) => app.paintShellCard(...a);
const refreshState = (...a) => app.refreshState(...a);

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

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

/* 状态栏横向滚动：内容超出 760px 列时给左右按钮、鼠标拖拽，触屏走原生滑动。
 * 按钮只在对应方向还有内容时出现，滚到两端就收起来。 */
let statusScroll = { inner: null, prev: null, next: null };

function statusScrollBtn(dir, label, inner, sign) {
  const b = doc.createElement('button');
  b.type = 'button';
  b.className = 'status-scroll-btn ' + dir;
  b.setAttribute('aria-label', label);
  b.appendChild(ui.icon('chevron-right'));
  b.onclick = () => {
    const view = doc.defaultView;
    const reduce = !!(view && view.matchMedia && view.matchMedia('(prefers-reduced-motion: reduce)').matches);
    inner.scrollBy({
      left: sign * Math.max(160, Math.round(inner.clientWidth * 0.6)),
      behavior: reduce ? 'auto' : 'smooth',
    });
  };
  return b;
}

function updateStatusScroll() {
  const { inner, prev, next } = statusScroll;
  if (!inner || !prev || !next) return;
  const max = inner.scrollWidth - inner.clientWidth;
  const can = inner.clientWidth > 0 && max > 1;
  prev.hidden = !can || inner.scrollLeft <= 1;
  next.hidden = !can || inner.scrollLeft >= max - 1;
}

/* 鼠标拖拽滚动；拖过 4px 就把这次 click 吞掉，避免拖动后误开 chip 的菜单。 */
function bindStatusDrag(inner) {
  let down = false, moved = false, startX = 0, startLeft = 0;
  inner.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'touch' || e.button !== 0) return;
    down = true; moved = false; startX = e.clientX; startLeft = inner.scrollLeft;
    inner.classList.add('dragging');
    if (inner.setPointerCapture) inner.setPointerCapture(e.pointerId);
  });
  inner.addEventListener('pointermove', (e) => {
    if (!down) return;
    const dx = e.clientX - startX;
    if (!moved && Math.abs(dx) < 4) return;
    moved = true;
    inner.scrollLeft = startLeft - dx;
    e.preventDefault();
  });
  const end = (e) => {
    if (!down) return;
    down = false;
    inner.classList.remove('dragging');
    if (inner.releasePointerCapture && e.pointerId != null) {
      try { inner.releasePointerCapture(e.pointerId); } catch (err) { /* 指针已失效 */ }
    }
  };
  inner.addEventListener('pointerup', end);
  inner.addEventListener('pointercancel', end);
  inner.addEventListener('click', (e) => {
    if (!moved) return;
    e.stopPropagation();
    e.preventDefault();
    moved = false;
  }, true);
  inner.addEventListener('scroll', updateStatusScroll, { passive: true });
  inner.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    inner.scrollBy({ left: (e.key === 'ArrowLeft' ? -1 : 1) * 160, behavior: 'smooth' });
  });
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
  inner.tabIndex = 0;
  inner.setAttribute('role', 'group');
  inner.setAttribute('aria-label', '会话状态，可横向滚动');
  bindStatusDrag(inner);

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
  const bar = doc.createElement('div');
  bar.className = 'session-status-bar';
  const prev = statusScrollBtn('prev', '向左滚动', inner, -1);
  const next = statusScrollBtn('next', '向右滚动', inner, 1);
  bar.appendChild(prev);
  bar.appendChild(inner);
  bar.appendChild(next);
  box.appendChild(bar);
  statusScroll = { inner, prev, next };
  updateStatusScroll();
  paintShellCard();
}

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, api, ui, app } = deps);
  ({ icon } = ui);
  if (g && g.addEventListener) g.addEventListener('resize', () => updateStatusScroll());
  Object.assign(root, { openPermissionMenu, openThinkingMenu, switchThinking, switchPermission, syncStatusbar });
}
