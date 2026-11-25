/**
 * 结构审查·权限值边界：`PermissionSet` 的 `profile` 是**它自己的事实**，外部只问谓词。
 * 目的：守住「同一概念只有一个权威表示」——外读字段就等于把判定规则（`profile == Online`）抄到第二处。
 * 判据：`permission` 单元之外（生产代码）出现 `.permissions.profile` 即失败；
 *   要问"能不能出站联网"就问 `allows_network()`，要换档只在 `apply_root` / `apply_narrowing` 里做。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");
const { unitDir } = require("./unitpath.js");

// 外部唯一允许的形态：谓词（`allows_network`）与合并入口（`apply_root` / `apply_narrowing`）。
const RAW_FIELD = /\.permissions\s*\.\s*profile\b/;

function rsFilesOf(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === "tests") continue;
      rsFilesOf(p, out);
    } else if (e.name.endsWith(".rs") && !/(^|_)tests\.rs$/.test(e.name)) out.push(p);
  }
  return out;
}

/** 权限值外读：返回问题列表（空 = 外部只问谓词）。 */
function permissionValues(root) {
  const problems = [];
  const base = path.join(root, "src");
  if (!fs.existsSync(base)) return problems;
  const own = unitDir(root, "permission") + path.sep;
  for (const file of rsFilesOf(base)) {
    if (file.startsWith(own)) continue;
    const lines = mask(fs.readFileSync(file, "utf8")).split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      if (!RAW_FIELD.test(lines[i])) continue;
      problems.push(
        "权限值：" + rel(root, file) + ":" + (i + 1) + " 在 permission 单元外直接读 `PermissionSet` 的字段——判定规则会在这里长出第二份，改成问谓词（`allows_network`），换档只在 `apply_root` / `apply_narrowing`：" + lines[i].trim()
      );
    }
  }
  return problems;
}

module.exports = { permissionValues };
