/**
 * 文件面门禁：写盘只有一处实现。
 * 判据：`src/` 下全部 .rs 的生产代码（剔除 `#[cfg(test)] mod` 与注释 / 字面量）里出现 `std::fs` 的变更原语
 *   或自己开写句柄（`File::create` / `OpenOptions::new`），且文件不在白名单，即失败。零豁免、没有基线文件可销账。
 * 白名单：`src/kernel/`（机制本体：`detail/fs.rs` 的写与事务、`detail/unpack.rs` 的流式解包、`detail/fs_meta.rs` 的属性位）。
 * 不管：`std::fs::read*` / `metadata` / `read_dir` / `symlink_metadata` 这类**读**原语（读不产生版本、不需要锁），
 *   以及 `kernel::api::fs` 面里的 `write` / `transact` / `stream_into` / `remove_tree` / `move_file` / `copy_file` / `ensure_dir`。
 * 反的是"各处自己拼原子写、自己拼锁、自己判越界"，不是禁用这些名字——写什么、准不准写留在调用方与策略。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { mask } = require("../../lib/rust-mask.js");

/** 变更原语（`std::fs::read*` / `metadata` / `read_dir` 这些读不在其中）。 */
const FS_PRIM =
  "(write|rename|remove_file|remove_dir|remove_dir_all|create_dir|create_dir_all|copy|hard_link|set_permissions|soft_link|symlink)";

/** 允许自己写盘的面：内核机制本体。 */
const ALLOWED_PREFIXES = ["src/kernel/"];

/** 测试专用文件（`#[cfg(test)]` 挂着的外置测试模块）不进判据：它们的 `std::fs` 是夹具。 */
const TEST_FILE = /(^|\/)(tests(\/|\.rs$)|test\.rs$)|_tests\.rs$/;

/** 目的：这个文件里的 `std::fs` 被叫什么——全路径、`use std::fs;` 的 `fs`、`use std::fs as x;` 的 x。 */
function fs_names(text) {
  const names = ["std::fs"];
  if (/^\s*use std::fs\s*;/m.test(text)) names.push("fs");
  for (const m of text.matchAll(/^\s*use std::fs as (\w+)\s*;/gm)) names.push(m[1]);
  if (/^\s*use std::fs::\{?[^}]*\bself\b/m.test(text)) names.push("fs");
  return names;
}

/** 目的：按这个文件里 `std::fs` 的叫法拼出判定正则。 */
function call_pattern(text) {
  const named = fs_names(text).map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const head = "\\b(?:" + named.join("|") + ")::" + FS_PRIM + "\\b";
  return new RegExp(head + "|\\bFile::create\\s*\\(|\\bOpenOptions::new\\s*\\(", "g");
}

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
function fsSurface(root) {
  const problems = [];
  let scanned = 0;
  for (const f of walk(root)) {
    const key = f.split(path.sep).join("/");
    if (TEST_FILE.test(key)) continue;
    if (ALLOWED_PREFIXES.some((p) => key.startsWith(p))) continue;
    scanned++;
    const text = mask(stripTestModules(fs.readFileSync(path.join(root, f), "utf8")));
    for (const m of text.matchAll(call_pattern(text))) {
      const line = text.slice(0, m.index).split("\n").length;
      problems.push(
        "文件面：" +
          key +
          ":" +
          line +
          " 直接用了 " +
          m[0].trim() +
          "——写盘只经 kernel::api::fs（锁、旧版本比对、暂存与一次发布都在那里）",
      );
    }
  }
  return {
    problems,
    notes: [
      "文件面：扫了 " + scanned + " 个生产源文件（已剔除测试模块与注释），自己写盘的面只有 src/kernel/",
    ],
  };
}

module.exports = { fsSurface };
