/**
 * 外部工具位置门禁：设置里那几个工具路径只有一个读者圈——位置解析（provision）与它的装配方。
 * 判据：`src` 里除 `units/catalog`（定义与校验）、`units/provision`（解析）、`units/runtime`（写入）、
 *   `units/work`（装配）与组合根 `entry` 之外，不得读 `qemu_path` / `virtiofsd_path` / `smbd_path`；
 *   别处要工具位置就问 `provision::api::vm_tools`，别再各解一遍（各自解析正是三条路各说各话的来源）。
 * 注释里提到字段名不算命中：先剥测试模块，再用 tests/lib/rust-mask.js 抹注释（保留字面量）。
 * 零豁免：命中即失败，没有基线文件可销账。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { maskComments } = require("../../lib/rust-mask.js");

/** 允许读这几个字段的目录：定义 / 校验、解析、写入、装配。 */
const ALLOWED = ["src/units/catalog/", "src/units/provision/", "src/units/runtime/", "src/units/work/", "src/entry/"];
/** 字段访问（`.qemu_path` 这种），不是字段声明。 */
const FIELD = /\.(qemu_path|virtiofsd_path|smbd_path)\b/g;
/** 测试自己的夹具，不按产品口径判。 */
function isTestFile(f) {
  if (f === "src/tests.rs" || f.startsWith("src/tests/")) return true;
  const segs = f.split("/");
  if (segs.includes("tests")) return true;
  const base = segs[segs.length - 1];
  return base === "tests.rs" || base.endsWith("_tests.rs");
}

function walk(root) {
  const files = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".rs")) {
        const f = rel(root, p);
        if (!isTestFile(f)) files.push(f);
      }
    }
  };
  walkDir(path.join(root, "src"));
  return files;
}

/** 目的：扫一遍，返回问题清单与一行说明（扫描面与豁免目录）。 */
function toolPaths(root) {
  const problems = [];
  let scanned = 0;
  for (const f of walk(root)) {
    if (ALLOWED.some((p) => f.startsWith(p))) continue;
    scanned++;
    const prod = maskComments(stripTestModules(fs.readFileSync(path.join(root, f), "utf8")));
    prod.split("\n").forEach((line, i) => {
      for (const m of line.matchAll(FIELD)) {
        problems.push(
          "工具路径：" + f + ":" + (i + 1) + " 直接读设置字段「" + m[1] +
            "」——改问 provision::api::vm_tools（位置解析只此一处）",
        );
      }
    });
  }
  return {
    problems,
    notes: ["工具路径：扫了 " + scanned + " 个源文件（豁免 " + ALLOWED.join(" / ") + "）"],
  };
}

module.exports = { toolPaths };
