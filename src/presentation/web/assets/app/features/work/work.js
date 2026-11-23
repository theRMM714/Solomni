/* 目的：会话的「工作」卡：提交与改动、上下文占用、工具与预算、历史提交的预览与还原。
 * 管：卡片与浮层的取数（GET .../work 等）、按需刷新（指纹比对）、上下文与工具浮层；提交历史的看与回滚见 commits.js。
 * 不管：状态栏里的那行速览（见 status-bar/）；确认框与提示（见 ui/dialog.js）。
 * 联动：由 app/main.js 装配；渲染与动作经 app.* 晚绑定（含 actions/ 与 transcript/ 的入口）。 */

import { workCopy } from './copy.js';

let doc, state, api, ui, app;
let btn;
const act = (...a) => app.act(...a);
const actionUrl = (...a) => app.actionUrl(...a);
const refreshState = (...a) => app.refreshState(...a);
const shellRunning = (...a) => app.shellRunning(...a);
const shellSecs = (...a) => app.shellSecs(...a);
const shellStatusLabel = (...a) => app.shellStatusLabel(...a);
const openWorkHistory = (...a) => app.openWorkHistory(...a);

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
  x.appendChild(ui.icon('close'));
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
  return [sh.command, sh.cwd, sh.started_at, sh.finished_at, sh.elapsed_secs,
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
  body.appendChild(popoverNote(sh.historical ? workCopy.historical : workCopy.fresh));
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

/* 过期占位符这一次做了什么：judged 是参与判定的过期读数，skipped = 省下的量不够付缓存失效。 */
function freshnessNote(stats) {
  if (!stats || !stats.judged) return workCopy.freshnessIdle;
  if (stats.skipped) return '本次判定 ' + stats.judged + ' 处：省下的量不够付一次缓存失效，没有替换。';
  return '本次替换 ' + stats.replaced + ' 处 · 省 ≈' + app.fmtTokens(stats.saved_tokens) + ' tokens。';
}

function openContextCard(s, anchor) {
  const box = popoverBox('上下文用量');
  const c = s.context;
  const u = s.usage_totals;
  const on = !!s.context_freshness;
  const keepPct = Number(s.freshness_keep_percent == null ? 80 : s.freshness_keep_percent);
  if (!c && !(u && u.calls)) {
    box.appendChild(popoverNote(workCopy.noContext));
    ui.popover(anchor, box, { label: '上下文用量' });
    return;
  }
  const at = Number((state.settings && state.settings.compact_at_percent) || 0);
  // 窗口登记为 0 = 本轮没拿到窗口：自动压缩按后端的 128K 兜底算，这里显示同一份事实。
  const win = Number((c && c.window) || 0) || 128000;
  const used = Number((c && c.total_tokens) || 0);
  const pct = c ? Math.min(100, (used / win) * 100) : null;
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
  box.appendChild(popoverRow('已用', c ? app.fmtTokens(used) + ' tokens' : '—'));
  box.appendChild(popoverRow(
    '窗口',
    c && c.window ? app.fmtTokens(c.window) + ' tokens' : app.fmtTokens(win) + ' tokens（兜底）',
  ));
  box.appendChild(popoverRow('占用', pct == null ? '—' : pct.toFixed(1) + '%'));
  if (u && u.calls) {
    box.appendChild(popoverRow('累计输入 / 输出', app.fmtTokens(u.prompt) + ' / ' + app.fmtTokens(u.completion)));
    // 缓存是**累计花费**的一部分：读/写都进 meta，命中率按累计读 / 累计输入算。
    const hit = app.cacheHitPercent(u.cache_read, u.prompt);
    const cache = ['读 ' + app.fmtTokens(u.cache_read)];
    if (hit != null) cache.push('命中 ' + Math.round(hit) + '%');
    if (Number(u.cache_creation) > 0) cache.push('写 ' + app.fmtTokens(u.cache_creation));
    box.appendChild(popoverRow('缓存（累计）', cache.join(' · ')));
    box.appendChild(popoverNote('共 ' + app.fmtInt(u.calls) + ' 次模型调用（含压缩）；回档不清零。'));
  } else {
    box.appendChild(popoverRow('累计输入 / 输出', '—'));
    box.appendChild(popoverNote(workCopy.noTotals));
  }
  box.appendChild(popoverRow('过期占位符', on ? '开 · 保底 ' + keepPct + '%' : '关'));
  if (on) box.appendChild(popoverNote(freshnessNote(s.freshness_stats)));
  // 压缩的三项是**全局设置**：收成一条可点的小字，点进设置中心，不再占三行。
  const keep = Number((state.settings && state.settings.compact_keep_percent) || 0);
  const link = doc.createElement('div');
  link.className = 'popover-note popover-link';
  link.textContent = '压缩：' + (at > 0 ? at + '% 触发' : '已关闭') + ' · ' +
    (keep > 0 ? '至少保留 ' + keep + '% 窗口' : '只留摘要') + ' · 总结模型 ' +
    ((state.settings && state.settings.compact_model) || '默认（会话同模型）');
  link.title = '点开设置中心「生成与上下文」改这三项';
  link.onclick = () => { ui.closePopover(); app.openSettingsHub('generate'); };
  box.appendChild(link);
  if (c) {
    const badge = doc.createElement('span');
    badge.className = 'badge-soft ' + (c.exact ? 'ok' : 'warn');
    badge.textContent = c.exact ? '供应商 usage（精确）' : '按字符估算';
    box.appendChild(badge);
    box.appendChild(popoverNote(c.exact
      ? '数字来自供应商返回的 usage；工具结果回灌的瞬间可能略小于下一请求。'
      : '供应商没回 usage：按对话字符 ÷ 4 估算，只作参考。'));
  }
  ui.popover(anchor, box, { label: '上下文用量' });
}

/* 手动压缩：协作主会话没有一条持续对话，由成员会话各自压。 */
function compactNow(s) {
  if (!s) return;
  if (s.flows) {
    ui.notice('有流程的会话不直接压缩', '协作主会话没有一条持续发给模型的对话；打开成员会话再用工具菜单压缩。', 'info');
    return;
  }
  act('compact');
}

/* 状态栏「工具」菜单：先放手动压缩；以后再加其它工具。 */
function openToolsMenu(chip, s) {
  const keep = Number((state.settings && state.settings.compact_keep_percent) || 0);
  const model = (state.settings && state.settings.compact_model) || '默认（会话同模型）';
  const blocked = s.flows
    ? '有流程的会话没有持续对话；成员会话各自压缩'
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
  ]);
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

export function mount(root, deps) {
  ({ doc, state, api, ui, app } = deps);
  btn = ui.btn;
  Object.defineProperty(root, "shellCardOpen", { get: () => shellCardOpen, set: (v) => { shellCardOpen = v; }, configurable: true, enumerable: true });
  Object.assign(root, { loadWork, popoverBox, popoverRow, popoverNote, shellCardSig, fillShellCard, paintShellCard, openShellCard, openContextCard, compactNow, openToolsMenu, openWorkCard });
}
