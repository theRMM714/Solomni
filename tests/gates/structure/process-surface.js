/**
 * 进程面门禁：跑外部程序只有一处实现。
 * 判据：`src/` 下全部 .rs 的生产代码（剔除 `#[cfg(test)] mod` 与注释 / 字面量）里出现 `std::process::Command::new`
 *   （或本地 `use std::process::Command` 之后的 `Command::new`），且文件不在白名单，即失败。零豁免、没有基线文件可销账。
 * 白名单：`src/kernel/`（机制本体：围栏执行 `detail/process.rs` 与宿主程序 `run_to_end`、长驻子进程 `detail/procs`、守门进程）。
 * 不管：`std::process::id()` / `exit()` / `Stdio` 这类不拉进程的原语。
 * 反的是"各处自己拉进程、自己判超时、自己拼杀树"，不是禁用这些名字——跑什么、给什么参数留在调用方。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { mask } = require("../../lib/rust-mask.js");

/** 允许自己拉进程的面：内核机制本体。 */
const ALLOWED_PREFIXES = ["src/kernel/"];

/** 测试专用文件（`#[cfg(test)]` 挂着的外置测试模块）不进判据：它们的进程是夹具。 */
const TEST_FILE = /(^|\/)(tests(\/|\.rs$)|test\.rs$)|_tests\.rs$/;

/** 目的：这个文件里的 `std::process::Command` 被叫什么——全路径、`use ...::{Command, …}`、`use ... as X`。 */
function command_names(text) {
  const names = ["std::process::Command"];
  if (/^\s*use std::process::(\{[^}]*\bCommand\b[^}]*\}|Command)\s*;/m.test(text)) names.push("Command");
  for (const m of text.matchAll(/^\s*use std::process::Command as (\w+)\s*;/gm)) names.push(m[1]);
  return names;
}

/** 目的：按这个文件里 `Command` 的叫法拼出判定正则。 */
function call_pattern(text) {
  const named = command_names(text).map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp("\\b(?:" + named.join("|") + ")::new\\s*\\(", "g");
}

/** 递归收集 src 下的 .rs。 */
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
function processSurface(root) {
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
        "进程面：" +
          key +
          ":" +
          line +
          " 直接用了 " +
          m[0].trim() +
          "——跑程序只经 kernel::api::process（run_to_end 的等待、超时与杀树都只有一处实现）",
      );
    }
  }
  return {
    problems,
    notes: [
      "进程面：扫了 " + scanned + " 个生产源文件（已剔除测试模块与注释），自己拉进程的面只有 src/kernel/",
    ],
  };
}

module.exports = { processSurface };
