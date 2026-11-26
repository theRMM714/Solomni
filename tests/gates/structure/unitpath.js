/**
 * 单元路径的唯一口径：单元 = `src/units/<桶>/<单元>/`；三个桶各收一类角色——环 1 的纯单元在 `pure/`，
 * 盘主在 `resource/`，其余业务单元在 `business/`。桶是「角色」不是环号：环是派生属性，写进路径后换环就是全量改名，
 * 还会与 layers.yaml 形成双权威，所以桶只按角色分，且每个单元恰好住一个桶（判据：tests/gates/structure/unit-buckets.js）。
 * 所有门禁认单元名都必须经这里，禁止再写 split("/")[2] 这类位次假设（桶多一层会让判据静默认错）。
 */
"use strict";
const fs = require("fs");
const path = require("path");

const PURE = "pure";
const RESOURCE = "resource";
const BUSINESS = "business";
const BUCKETS = new Set([PURE, RESOURCE, BUSINESS]);

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

/** `crate::units::[桶::]<单元>::…` → 单元名；不是单元路径返回空串。 */
function unitOfModule(full) {
  const m = /^crate::units::(?:(?:pure|resource|business)::)?([a-z_][a-z0-9_]*)/.exec(String(full));
  return m ? m[1] : "";
}

/** 单元在盘上的目录：按桶找（每个单元恰好住一个桶）。 */
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

module.exports = { PURE, RESOURCE, BUSINESS, BUCKETS, unitOfPath, unitOfModule, unitDir, unitPrefix };
