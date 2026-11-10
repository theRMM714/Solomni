/* 目的：纯字符串与数值格式化，供各层共用（不读 DOM、不碰状态）。
 * 管：正则转义、字节 / 数字 / 时长 / 时间显示、纯文本转义、单行截断、上下文占比。
 * 不管：任何业务判断、任何状态读取。
 * 联动：shortPath 用 escapeRe；状态栏、会话标签、存储页读这里的显示函数。
 */

/// 正则特殊字符转义：根路径里可能有 . + ( 之类。
export function escapeRe(s) {
  const special = '\\^$.*+?()[]{}|';
  return String(s).split('').map((c) => (special.indexOf(c) >= 0 ? '\\' + c : c)).join('');
}

export function fmtBytes(n) {
  const v = Number(n) || 0;
  if (v >= 1024 * 1024 * 1024) return (v / (1024 * 1024 * 1024)).toFixed(1) + ' GiB';
  if (v >= 1024 * 1024) return (v / (1024 * 1024)).toFixed(1) + ' MiB';
  if (v >= 1024) return (v / 1024).toFixed(1) + ' KiB';
  return v + ' B';
}

/// 纯文本转义：工具卡片的原始 JSON 一律按字面显示（<pre> 里不解释 HTML）。
export function escHtml(s) {
  return String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (
    { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]
  ));
}

/** 一行预览：把多行压成一行，过长截断（卡片 / 清单用，不冒充全文）。 */
export function oneLine(text, max) {
  const s = String(text == null ? '' : text).replace(/\s+/g, ' ').trim();
  const n = max || 80;
  return s.length > n ? s.slice(0, n) + '…' : s;
}

export function fmtInt(n) {
  const v = Number(n || 0);
  return Number.isFinite(v) ? v.toLocaleString('zh-CN') : '0';
}

export function ctxPercent(ctx) {
  if (!ctx || !ctx.window) return null;
  return (Number(ctx.total_tokens || 0) / Number(ctx.window)) * 100;
}

export function fmtDuration(sec) {
  const s = Math.max(0, Math.floor(Number(sec) || 0));
  if (s < 60) return s + ' 秒';
  const m = Math.floor(s / 60);
  const r = s % 60;
  return m + ' 分' + (r ? r + ' 秒' : '');
}

export function timeAgo(ts) {
  const t = Number(ts || 0);
  if (!t) return '';
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - t);
  if (diff < 60) return '刚刚';
  if (diff < 3600) return Math.floor(diff / 60) + ' 分钟前';
  if (diff < 86400) return Math.floor(diff / 3600) + ' 小时前';
  if (diff < 30 * 86400) return Math.floor(diff / 86400) + ' 天前';
  return fmtDateTime(t);
}

export function fmtDateTime(ts) {
  const t = Number(ts || 0);
  if (!t) return '—';
  try {
    return new Date(t * 1000).toLocaleString('zh-CN', { hour12: false });
  } catch (e) { return String(t); }
}
