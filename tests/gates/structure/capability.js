/**
 * 能力内部结构门禁：四条硬判据。文件粒度（行数、内联测试）由 structure/size.js 一处判，这里不管。
 * 硬判据：① 同能力 service 不得引用本能力 detail 或 kernel::detail；② service 不得直接做 IO；
 *        ③ 同一能力的 service 方法族之间不得成环；④ 能力内 detail 之外的文件不得引用 kernel::detail。
 * 覆盖 api / service / domain / detail 四个分区（api 含 api.rs 与 api/ 子目录）。
 * 判据只看生产代码：先剔 #[cfg(test)] 模块，再用 tests/lib/rust-mask.js 抹掉注释与字面量，
 *   避免文档注释里提到 ureq / std::fs 就被算成 IO 调用。
 * 零豁免：实际命中即失败，没有基线文件可销账。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { mask } = require("../../lib/rust-mask.js");

const IO_CALLS = ["std::fs", "std::process::Command", "ureq", "reqwest", "std::net"];
/** 端口方法之外的 canonicalize 调用也是直接 IO：`probe.canonicalize` 走端口，`path.canonicalize` 直接碰盘。 */
const CANONICALIZE = /canonicalize\s*\(/;

/** 能力内的实现分区：api / service / domain / detail；其余（能力根 X.rs、guest.rs 这类）不归这些判据管。 */
function scopeOf(f) {
  const parts = f.split("/");
  if (parts[0] !== "src" || parts[1] !== "capabilities" || parts.length < 4) return null;
  const rest = parts.slice(3).join("/");
  if (rest === "api.rs" || rest.startsWith("api/")) return "api";
  for (const name of ["service", "domain", "detail"]) {
    if (rest === name + ".rs" || rest.startsWith(name + "/")) return name;
  }
  return null;
}

/** 同能力 service 下的方法族：它引用的兄弟模块（只看生产代码边）。 */
function siblingEdges(f, text) {
  const self = f.split("/").pop().replace(/\.rs$/, "");
  const names = new Set();
  for (const m of text.matchAll(/\bsuper::([a-z_][a-z0-9_]*)/g)) names.add(m[1]);
  for (const m of text.matchAll(/crate::capabilities::[a-z_][a-z0-9_]*::service::([a-z_][a-z0-9_]*)/g)) names.add(m[1]);
  names.delete(self);
  names.delete("self");
  names.delete("super");
  return names;
}

function tarjanScc(graph) {
  const idx = {}, low = {}, on = {}, stack = [];
  const comps = [];
  let counter = 0;
  const visit = (v) => {
    idx[v] = low[v] = counter++;
    stack.push(v);
    on[v] = true;
    for (const w of graph[v] || []) {
      if (!(w in idx)) { visit(w); low[v] = Math.min(low[v], low[w]); }
      else if (on[w]) low[v] = Math.min(low[v], idx[w]);
    }
    if (low[v] === idx[v]) {
      const comp = [];
      let w;
      do { w = stack.pop(); on[w] = false; comp.push(w); } while (w !== v);
      if (comp.length > 1) comps.push(comp.sort());
    }
  };
  for (const v of Object.keys(graph)) if (!(v in idx)) visit(v);
  return comps;
}

function walk(root) {
  const files = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".rs")) files.push(rel(root, p));
    }
  };
  walkDir(path.join(root, "src", "capabilities"));
  return files;
}

/** 扫描：返回每条判据的实际命中与文件粒度快照，供门禁比对与基线重新生成共用。 */
function scan(root) {
  const serviceToDetail = new Set();
  const outsideDetailKernel = new Set();
  const serviceIo = new Set();
  const serviceGraph = {};
  for (const f of walk(root)) {
    const scope = scopeOf(f);
    if (!scope) continue;
    const raw = fs.readFileSync(path.join(root, f), "utf8");
    const prod = mask(stripTestModules(raw));
    const cap = f.split("/")[2];

    if (scope !== "detail" && prod.includes("crate::kernel::detail")) {
      outsideDetailKernel.add(f + " -> crate::kernel::detail");
    }

    if (scope === "service") {
      if (prod.includes("crate::capabilities::" + cap + "::detail")) {
        serviceToDetail.add(f + " -> crate::capabilities::" + cap + "::detail");
      }
      if (prod.includes("crate::kernel::detail")) serviceToDetail.add(f + " -> crate::kernel::detail");
      for (const call of IO_CALLS) {
        if (prod.includes(call)) serviceIo.add(f + " -> " + call);
      }
      const withoutPort = prod.replace(/\b(?:self\s*\.\s*)?probe\s*\.\s*canonicalize\s*\(/g, "");
      if (CANONICALIZE.test(withoutPort)) serviceIo.add(f + " -> Path::canonicalize");
      const node = "capabilities/" + cap + "/service/" + f.split("/").pop();
      serviceGraph[node] = siblingEdges(f, prod);
    }
  }
  const cycles = tarjanScc(serviceGraph).map((c) => "capabilities/" + c.join(", "));
  return { serviceToDetail: [...serviceToDetail].sort(), outsideDetailKernel: [...outsideDetailKernel].sort(), serviceIo: [...serviceIo].sort(), cycles };
}

function internal(root) {
  const actual = scan(root);
  const problems = [];
  const fail = (hint, list) => {
    for (const a of list) problems.push("能力内部：" + hint + "：" + a);
  };
  fail("service 不得引用本能力 detail 或 kernel::detail（抽端口，别直连实现）", actual.serviceToDetail);
  fail("detail 之外的能力文件不得引用 kernel::detail（由组合根注入端口）", actual.outsideDetailKernel);
  fail("service 不得直接做 IO（fs / Command / 网络 / canonicalize）；经端口或 detail", actual.serviceIo);
  for (const c of actual.cycles) problems.push("能力内部：同能力 service 方法族成环：" + c);
  return { problems, notes: [] };
}

module.exports = { internal, scan };
