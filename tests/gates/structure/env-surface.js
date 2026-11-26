/**
 * 环境面门禁：进程环境只有一处读取面。
 * 判据：`src/` 下全部 .rs 的生产代码（剔除 `#[cfg(test)] mod` 与注释 / 字面量）里出现 `std::env::` 的读写原语，
 *   且文件不在白名单，即失败。零豁免、没有基线文件可销账。
 * 白名单：`src/kernel/detail/io/env.rs`（机制本体）、`src/main.rs`（启动序列）、`src/entry/`（入口装配）、
 *   `src/presentation/`（终端能力探测与回退：NO_COLOR / TERM 一类是呈现面事实，不是业务事实）。
 * 不管：`std::env::consts`（编译期常量，不是 IO）与 `split_paths` / `join_paths`（纯字符串解析）。
 * 反的是"各处自己摸进程环境"，不是禁用这些名字——值怎么解释留在调用方。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { mask } = require("../../lib/rust-mask.js");

/** 直接读写的原语（大小写敏感；`consts` / `split_paths` / `join_paths` 不在其中）。 */
const ENV_CALL =
  /std::env::(var|var_os|temp_dir|current_dir|current_exe|set_var|remove_var|args|home_dir)\b/g;

/** 允许直接摸进程环境的面。 */
const ALLOWED = ["src/kernel/detail/io/env.rs", "src/main.rs"];
const ALLOWED_PREFIXES = ["src/entry/", "src/presentation/"];

/** 测试专用文件（`#[cfg(test)]` 挂着的外置测试模块）不进判据：它们的 `std::env` 是夹具。 */
const TEST_FILE = /(^|\/)(tests(\/|\.rs$)|test\.rs$)|_tests\.rs$/;

/** 递归收集 src 下的 .rs（测试专用文件与内联测试模块在判定前剔除）。 */
function walk(root) {
  const out = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".rs")) out.push(rel(root, p));
    }
  };
  walkDir(path.join(root, "src"));
  return out;
}

/** 目的：扫一遍，返回问题清单与一行说明。 */
function envSurface(root) {
  const problems = [];
  let scanned = 0;
  for (const f of walk(root)) {
    const key = f.split(path.sep).join("/");
    if (TEST_FILE.test(key) || ALLOWED.includes(key)) continue;
    if (ALLOWED_PREFIXES.some((p) => key.startsWith(p))) continue;
    scanned++;
    const text = mask(stripTestModules(fs.readFileSync(path.join(root, f), "utf8")));
    for (const m of text.matchAll(ENV_CALL)) {
      const line = text.slice(0, m.index).split("\n").length;
      problems.push(
        "环境面：" +
          key +
          ":" +
          line +
          " 直接用了 " +
          m[0] +
          "——进程环境只由 kernel/detail/io/env.rs 读，单元与内核机制经 kernel::api::env 取",
      );
    }
  }
  return {
    problems,
    notes: [
      "环境面：扫了 " +
        scanned +
        " 个生产源文件（已剔除测试模块与注释），直接摸 std::env 的面只有 kernel/detail/io/env.rs、main.rs、entry/、presentation/",
    ],
  };
}

module.exports = { envSurface };
