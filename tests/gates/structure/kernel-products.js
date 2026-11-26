/**
 * 内核产品名门禁：内核只提供通用机制，不认识任何具体产品。
 * 判据：`src/kernel` 下的源码（含测试与注释）不得出现产品名 `qemu` / `smbd` / `virtiofsd`
 *   （`smbd.conf` 由 `smbd` 一并覆盖）。命中即失败，零豁免、没有基线文件可销账。
 * 不管：参数名、共享名、环境变量名里的同形片段——反的是"内核知道有这么个产品"，不是禁用这些字眼；
 *   产品自己的 argv / 就地协议 / 配置文件名留在单元里，见 `docs/architecture/ports.md` 的进程面。
 * 事件归因靠 `purpose` / `marker` 这类中性标签（`guest`、`share:smb`，见 `src/kernel/detail/procs.rs`），
 *   不是靠内核认产品名。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");

/** 产品名（大小写不敏感）：产品自己的可执行文件名与配置文件名。 */
const PRODUCTS = ["qemu", "smbd", "virtiofsd"];

/** 递归收集 src/kernel 下的 .rs（测试与注释一起判：内核不该在任何地方提到产品）。 */
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
  walkDir(path.join(root, "src", "kernel"));
  return out;
}

/** 目的：扫一遍，返回问题清单与一行说明（扫了多少文件）。 */
function kernelProducts(root) {
  const problems = [];
  const files = walk(root);
  for (const f of files) {
    const text = fs.readFileSync(path.join(root, f), "utf8");
    text.split("\n").forEach((line, i) => {
      const lower = line.toLowerCase();
      for (const name of PRODUCTS) {
        if (!lower.includes(name)) continue;
        problems.push(
          "内核产品名：" + f + ":" + (i + 1) + " 出现了产品名「" + name + "」——内核只提供通用机制，产品名与它的 argv / 配置文件名留在单元里",
        );
      }
    });
  }
  return {
    problems,
    notes: ["内核产品名：扫了 " + files.length + " 个内核源文件，禁 " + PRODUCTS.join(" / ")],
  };
}

module.exports = { kernelProducts };
