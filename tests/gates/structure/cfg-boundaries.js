/**
 * 结构审查·平台 cfg 边界：平台差异只允许留在三种地方——平台后端文件、平台事实模块、平台测试。
 * 其余位置只允许「后端选择」：紧跟着 mod / use 的 cfg 属性（模块别名、重导出）。
 * 目的：防止 #[cfg(windows)] / cfg!(target_os = …) 重新散落进公共路径；零豁免。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");

const BACKEND_NAMES = new Set(["windows", "linux", "macos", "unix", "other"]);
const PLATFORM = /(^|[^A-Za-z0-9_])(windows|unix|linux|macos|target_os|target_family|target_arch|target_env|target_pointer_width)([^A-Za-z0-9_]|$)/;
const CFG_ATTR = /^\s*#!?\[(?:cfg|cfg_attr)\s*\(/;
const WHOLE_FILE_CFG = /^\s*#!\[cfg\s*\(/;
const CFG_EXPR = /\bcfg!\s*\(/;
const SELECTION = /^\s*(?:pub(?:\((?:crate|super)\))?\s+)?(?:mod|use)\b/;

/** 平台后端文件：文件名或任一目录名是平台名。 */
function isBackendFile(r) {
  const parts = r.split("/");
  const stem = parts[parts.length - 1].replace(/\.rs$/, "");
  if (BACKEND_NAMES.has(stem)) return true;
  return parts.slice(0, -1).some((p) => BACKEND_NAMES.has(p));
}

/** 允许出现平台 cfg 的文件：后端、平台事实、平台测试（含同目录的 tests.rs）。 */
function isAllowedFile(r) {
  if (r === "src/tests.rs" || r.startsWith("src/tests/")) return true;
  if (r.endsWith("/tests.rs")) return true;
  if (r === "src/kernel/domain/platform.rs") return true;
  return isBackendFile(r);
}

/** 下一个非空、非注释、非属性的代码行。 */
function nextCodeLine(lines, i) {
  for (let j = i + 1; j < lines.length; j++) {
    const t = lines[j].trim();
    if (!t) continue;
    if (t.startsWith("//") || t.startsWith("/*") || t.startsWith("*")) continue;
    if (/^#!?\[/.test(t)) continue;
    return t;
  }
  return "";
}

/** 这个 cfg 属性是不是某个测试项上的（后续属性里有 #[test]）。 */
function isTestItem(lines, i) {
  for (let j = i + 1; j < lines.length; j++) {
    const t = lines[j].trim();
    if (!t) continue;
    if (t.startsWith("//") || t.startsWith("/*") || t.startsWith("*")) continue;
    if (/^#!?\[/.test(t)) {
      if (/^#\[(?:[a-z_]+::)?test\b/.test(t)) return true;
      continue;
    }
    return false;
  }
  return false;
}

/** 行内 #[cfg(test)] mod tests { … } 的行区间：这些行是测试，不是生产平台分支。 */
function inlineTestLines(lines) {
  const skip = new Array(lines.length).fill(false);
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) continue;
    let j = i + 1;
    while (j < lines.length && lines[j].trim() === "") j++;
    if (j >= lines.length) continue;
    if (!/^\s*(?:pub\s+)?mod\s+\w+\s*\{/.test(lines[j])) continue;
    let depth = 0;
    let started = false;
    for (let k = j; k < lines.length; k++) {
      for (const ch of lines[k]) {
        if (ch === "{") {
          depth++;
          started = true;
        } else if (ch === "}") {
          depth--;
        }
      }
      skip[k] = true;
      if (started && depth <= 0) break;
    }
    i = j;
  }
  return skip;
}

/** 读整条属性（多行 cfg 时续到闭合的 ]）。 */
function attrText(lines, i) {
  let text = lines[i];
  if (text.includes("]")) return text;
  for (let j = i + 1; j < lines.length && j - i < 8; j++) {
    text += " " + lines[j];
    if (text.includes("]")) break;
  }
  return text;
}

function boundaries(root) {
  const problems = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) {
        walk(p);
        continue;
      }
      if (!e.name.endsWith(".rs")) continue;
      const r = rel(root, p);
      if (isAllowedFile(r)) continue;
      const lines = mask(fs.readFileSync(p, "utf8")).split(/\r?\n/);
      const inTestModule = inlineTestLines(lines);
      for (let i = 0; i < lines.length; i++) {
        if (inTestModule[i]) continue;
        const line = lines[i];
        if (WHOLE_FILE_CFG.test(line)) {
          problems.push(`${r}:${i + 1} 整文件平台门只允许出现在后端文件里：${line.trim()}`);
          continue;
        }
        const attr = CFG_ATTR.test(line);
        const expr = CFG_EXPR.test(line);
        if (!attr && !expr) continue;
        const full = attr ? attrText(lines, i) : line;
        if (!PLATFORM.test(full)) continue;
        // lint 注解不构成平台分支（allow / warn / deny / expect 只改告警）。
        if (attr && /^\s*#!?\[cfg_attr\s*\(/.test(line) && /\b(allow|warn|deny|expect)\s*\(/.test(full)) continue;
        if (isTestItem(lines, i)) continue;
        if (attr && SELECTION.test(nextCodeLine(lines, i))) continue;
        problems.push(
          `${r}:${i + 1} 平台 cfg 只能出现在后端文件 / 平台事实 / 平台测试，或紧邻 mod / use 的后端选择里：${line.trim()}`
        );
      }
    }
  };
  walk(path.join(root, "src"));
  return problems;
}

module.exports = { boundaries };
