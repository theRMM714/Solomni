/* 目的：顶部状态栏用的展示派生：上下文占用档位、shell 回执怎么说、跑了多久、这次调用的权限/模型摘要。
 * 管：只读快照字段算出来的文字与配色档（纯函数，不碰 DOM）。
 * 不管：状态栏的布局与刷新时机（见 status-bar/status-bar.js 的 syncStatusbar）。
 * 联动：由 app/main.js 装配；settings 来自 app/stores/state.js，长时任务的时间基准是回包的 started_at。 */

let g, doc, state, stores, api, ui, app;

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

export function mount(root, deps) {
  ({ g, doc, state, stores, api, ui, app } = deps);
  Object.assign(root, { ctxChipClass, shellRunning, shellAbnormal, shellStatusLabel, shellSecs, statusPrintOf });
}
