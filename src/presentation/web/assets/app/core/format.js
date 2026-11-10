/* 目的：纯字符串与数值格式化，供各层共用（不读 DOM、不碰状态）。
 * 管：正则转义 escapeRe、字节数显示 fmtBytes。
 * 不管：任何业务判断、任何状态读取。
 * 联动：app.js 的 shortPath 用 escapeRe；设置中心的存储页用 fmtBytes。
 */

/// 正则特殊字符转义：根路径里可能有 . + ( 之类。
function escapeRe(s) {
  const special = '\\^$.*+?()[]{}|';
  return String(s).split('').map((c) => (special.indexOf(c) >= 0 ? '\\' + c : c)).join('');
}

function fmtBytes(n) {
  const v = Number(n) || 0;
  if (v >= 1024 * 1024 * 1024) return (v / (1024 * 1024 * 1024)).toFixed(1) + ' GiB';
  if (v >= 1024 * 1024) return (v / (1024 * 1024)).toFixed(1) + ' MiB';
  if (v >= 1024) return (v / 1024).toFixed(1) + ' KiB';
  return v + ' B';
}
