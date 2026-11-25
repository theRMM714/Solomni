/**
 * 单元路径的唯一口径：单元 = `src/units/<单元>/`；环 1 的单元在 `src/units/pure/<单元>/`。
 * 环 1 放进 pure/ 是硬不变量的路径编码（纯层无 IO、无状态、无端口），其它环扁平——
 *   环号本身不写进路径：环是派生属性，写进路径后换环就是全量改名，还会与 layers.yaml 形成双权威。
 * 所有门禁认单元名都必须经这里，禁止再写 split("/")[2] 这类位次假设（pure/ 多一层会让判据静默认错）。
 */
"use strict";
const fs = require("fs");
const path = require("path");

const PURE = "pure";

/** 仓库相对或绝对路径 → 单元名；不在单元目录下返回空串。 */
function unitOfPath(file) {
  const parts = String(file).replace(/\\/g, "/").split("/");
  const at = parts.indexOf("units");
  if (at < 0 || parts[at + 1] === undefined) return "";
  const i = parts[at + 1] === PURE ? at + 2 : at + 1;
  const seg = parts[i];
  if (!seg) return "";
  return seg.replace(/\.rs$/, "");
}

/** `crate::units::[pure::]<单元>::…` → 单元名；不是单元路径返回空串。 */
function unitOfModule(full) {
  const m = /^crate::units::(?:pure::)?([a-z_][a-z0-9_]*)/.exec(String(full));
  return m ? m[1] : "";
}

/** 单元在盘上的目录：环 1 在 pure/ 下，其余扁平。 */
function unitDir(root, name) {
  const p = path.join(root, "src", "units", PURE, name);
  return fs.existsSync(p) ? p : path.join(root, "src", "units", name);
}

/** 单元的模块前缀段：["crate","units"(, "pure"),<单元>]，给「单元内文件之间」的边解析用。 */
function unitPrefix(root, name) {
  return fs.existsSync(path.join(root, "src", "units", PURE, name))
    ? ["crate", "units", PURE, name]
    : ["crate", "units", name];
}

module.exports = { PURE, unitOfPath, unitOfModule, unitDir, unitPrefix };
