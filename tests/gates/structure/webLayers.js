/**
 * 前端分层门禁：assets/app/ 是原生 ESM 分层（core / stores / ui / features），依赖方向只由目录名判定。
 * 硬判据：① core 只依赖 core；② stores 只依赖 core，ui 只依赖 core 与 ui 自身；③ features/<功能> 只依赖 core / stores / ui 与自己的目录；
 *        ④ 只有 main.js 可以 import 全部层；⑤ 不许再引已删除的 app.js / bridge.js，也不许出现裸模块名（无 npm 依赖）。
 * 回执：模块文件超过 400 行按行列列出，不改变成败——行数是气味，依赖方向才是规则。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");

const APP = "src/presentation/web/assets/app";
const SOFT_LIMIT = 400;

/** 入口文件：装配点，唯一知道全部功能的地方。 */
const ENTRY = "entry";

/** 文件（app/ 内的相对路径）到层名；未知目录即失败。 */
function layerOf(inner) {
  if (inner === "main.js") return ENTRY;
  const parts = inner.split("/");
  if (parts[0] === "core") return "core";
  if (parts[0] === "stores") return "stores";
  if (parts[0] === "ui") return "ui";
  if (parts[0] === "features" && parts.length > 2) return "feature:" + parts[1];
  return null;
}

/** 允许的 import 目标层；功能只许停留在自己的目录里，功能之间互不 import。 */
function allows(fromLayer, toLayer) {
  if (fromLayer === ENTRY) return true;
  if (fromLayer === "core") return toLayer === "core";
  if (fromLayer === "stores") return toLayer === "core";
  if (fromLayer === "ui") return toLayer === "core" || toLayer === "ui";
  if (fromLayer.startsWith("feature:")) {
    return toLayer === "core" || toLayer === "stores" || toLayer === "ui" || toLayer === fromLayer;
  }
  return false;
}

/** 静态 import / export-from 与副作用 import 的模块说明符（多行具名列表也算；注释先剥掉）。 */
function specifiers(text) {
  const clean = text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
  const out = [];
  const at = (index) => clean.slice(0, index).split(/\r?\n/).length;
  for (const m of clean.matchAll(/(?:^|\n)[ \t]*(?:import|export)[^"'\u0060]*?\bfrom\s*["']([^"']+)["']/g)) {
    out.push({ spec: m[1], line: at(m.index) });
  }
  for (const m of clean.matchAll(/(?:^|\n)[ \t]*import\s*["']([^"']+)["']/g)) {
    out.push({ spec: m[1], line: at(m.index) });
  }
  return out;
}

function files(root) {
  const out = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".js")) out.push(rel(root, p));
    }
  };
  walkDir(path.join(root, APP));
  return out.sort();
}

function webLayers(root) {
  const problems = [];
  const notes = [];
  const all = files(root);
  if (!all.length) return { problems: [APP + " 下没有任何模块（前端分层已不存在？）"], notes };

  for (const f of all) {
    const inner = f.slice(APP.length + 1);
    const from = layerOf(inner);
    if (!from) { problems.push(f + " 不在 core / stores / ui / features 任一目录下，也不在 app/main.js：分层无归属"); continue; }
    const text = fs.readFileSync(path.join(root, f), "utf8");
    const lineCount = text.split(/\r?\n/).length;
    if (lineCount > SOFT_LIMIT) notes.push(f + " 共 " + lineCount + " 行（软线 " + SOFT_LIMIT + "），考虑按职责再拆");

    for (const { spec, line } of specifiers(text)) {
      if (!spec.startsWith(".")) {
        problems.push(f + ":" + line + " 引用了裸模块名 " + JSON.stringify(spec) + "：前端没有 npm 依赖，只用相对路径");
        continue;
      }
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(inner), spec));
      if (target === "app.js" || target === "bridge.js" || target.endsWith("/app.js")) {
        problems.push(f + ":" + line + " 引用了已删除的 " + target + "：前端入口只有 app/main.js");
        continue;
      }
      const to = layerOf(target);
      if (!to) {
        problems.push(f + ":" + line + " 指向 " + target + "，它不在分层目录里");
        continue;
      }
      if (!allows(from, to)) {
        problems.push(f + ":" + line + " 从 " + from + " 依赖 " + to + "（" + target + "）：依赖方向只允许 core ← stores ← ui ← features，功能之间互不 import");
      }
    }
  }

  // 入口仍由 index.html 以 module 方式单独加载；旧的整体脚本与迁移期桥都不许回来。
  const html = path.join(root, "src/presentation/web/assets/index.html");
  if (fs.existsSync(html)) {
    const h = fs.readFileSync(html, "utf8");
    if (/src=["']\/assets\/app\.js["']/.test(h)) problems.push("assets/index.html 仍在加载 /assets/app.js（已删除）");
    if (!/type=["']module["'][^>]*src=["']\/assets\/app\/main\.js["']|src=["']\/assets\/app\/main\.js["'][^>]*type=["']module["']/.test(h)) {
      problems.push("assets/index.html 没有以 type=\"module\" 加载 /assets/app/main.js：页面只该有这一个入口");
    }
  } else {
    problems.push("assets/index.html 不存在：页面挂载点丢失");
  }
  return { problems, notes };
}

module.exports = { webLayers };
