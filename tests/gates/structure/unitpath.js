/**
 * 单元路径的唯一口径：单元 = `src/units/<单元>/`；角色桶把同类单元再收一层——环 1 在 `src/units/pure/<单元>/`，
 * 盘主在 `src/units/resource/<单元>/`。桶是「角色」不是环号：环是派生属性，写进路径后换环就是全量改名，
 * 还会与 layers.yaml 形成双权威，所以只有这两类真正成桶。
 * 所有门禁认单元名都必须经这里，禁止再写 split("/")[2] 这类位次假设（桶多一层会让判据静默认错）。
 */
"use strict";
const fs = require("fs");
const path = require("path");

const PURE = "pure";
const RESOURCE = "resource";
const BUCKETS = new Set([PURE, RESOURCE]);

/** 仓库相对或绝对路径 → 单元名；不在单元目录下返回空串。 */
function unitOfPath(file) {
  const parts = String(file).replace(/\\/g, "/").split("/");
  const at = parts.indexOf("units");
  if (at < 0 || parts[at + 1] === undefined) return "";
  const i = BUCKETS.has(parts[at + 1]) ? at + 2 : at + 1;
  const seg = parts[i];
  if (!seg) return "";
  return seg.replace(/\.rs$/, "");
}

/** `crate::units::[pure::|resource::]<单元>::…` → 单元名；不是单元路径返回空串。 */
function unitOfModule(full) {
  const m = /^crate::units::(?:(?:pure|resource)::)?([a-z_][a-z0-9_]*)/.exec(String(full));
  return m ? m[1] : "";
}

/** 单元在盘上的目录：先找桶，再找扁平单元。 */
function unitDir(root, name) {
  for (const bucket of BUCKETS) {
    const p = path.join(root, "src", "units", bucket, name);
    if (fs.existsSync(p)) return p;
  }
  return path.join(root, "src", "units", name);
}

/** 单元的模块前缀段：["crate","units"(, 桶),<单元>]，给「单元内文件之间」的边解析用。 */
function unitPrefix(root, name) {
  for (const bucket of BUCKETS) {
    if (fs.existsSync(path.join(root, "src", "units", bucket, name))) return ["crate", "units", bucket, name];
  }
  return ["crate", "units", name];
}

module.exports = { PURE, RESOURCE, BUCKETS, unitOfPath, unitOfModule, unitDir, unitPrefix };
