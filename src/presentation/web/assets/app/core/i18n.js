/* 目的：回执说明的界面词表——键是稳定码，值是后端从 `locales/<lang>.ftl` 导出的单行模板。
 * 管：收一份目录；把一条说明（码 + 参数）替换成一句人读文本。
 * 不管：文案的权威（在 `locales/` 与后端）；界面自己的词表（各 feature 的 `copy.js`）；语言协商。
 * 联动：`projectSnapshot` 写入 `/api/state` 的 `i18n`；转录的工具卡片按它渲染 notes。 */

let catalog = {};
let lang = "zh-CN";

/** 收一份目录（`/api/state` 的 i18n）：语言标签 + 码到模板的映射。 */
export function setCatalog(next) {
  lang = (next && next.lang) || "zh-CN";
  catalog = (next && next.messages) || {};
}

/** 当前界面语言标签（与后端协商出来的那一份一致）。 */
export function currentLang() { return lang; }

const ARG = /\{\s*\$([A-Za-z0-9_]+)\s*\}/g;

/** 一条说明 → 文本；目录里没有这条码时退回 `[code]`（显眼但不崩）。 */
export function noteText(note) {
  const n = note || {};
  const tpl = catalog[n.code];
  if (tpl == null) return "[" + (n.code || "?") + "]";
  return String(tpl).replace(ARG, (m, k) => {
    const v = n.args && n.args[k];
    return v == null ? m : String(v);
  });
}

/** 一组说明 → 多行文本（没有说明就是空串）。 */
export function notesText(notes) {
  return (notes || []).map(noteText).join("\n");
}
