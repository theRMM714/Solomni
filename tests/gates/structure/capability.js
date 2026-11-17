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

/** 能力内的实现分区：api / service / domain / detail；其余（能力根 X.rs、guest.rs 这类）不归这些判据管。
 *  单元与子业务之间可以多一层目录（如 capabilities/conversation/session/domain），判据取路径里第一个分区段。 */
function scopeOf(f) {
  const parts = f.split("/");
  if (parts[0] !== "src" || parts[1] !== "capabilities" || parts.length < 4) return null;
  for (let i = 3; i < parts.length; i += 1) {
    const seg = parts[i].replace(/\.rs$/, "");
    if (["api", "service", "domain", "detail"].includes(seg)) return seg;
  }
  return null;
}

/** 一个 service 文件的模块路径：`capabilities/<单元>/<子业务…>/service/<rest>`（rest 不含 .rs，可含子目录）。 */
function modulePathOf(f) {
  return "capabilities/" + f.split("/").slice(2).join("/").replace(/\.rs$/, "");
}

/** 同能力 service 下的方法族：它引用的兄弟模块（只看生产代码边，只留真实存在的模块）。 */
function siblingEdges(mod, text, nodes) {
  const dir = mod.slice(0, mod.lastIndexOf("/"));
  const out = new Set();
  const pick = (candidate) => {
    if (candidate !== mod && nodes.has(candidate)) out.add(candidate);
  };
  if (/\bsuper::\*/.test(text)) pick(dir);
  for (const m of text.matchAll(/\bsuper::([a-z_][a-z0-9_]*)/g)) {
    if (m[1] !== "self" && m[1] !== "super") pick(dir + "/" + m[1]);
  }
  const at = mod.indexOf("/service/");
  if (at < 0) return out;
  const base = mod.slice(0, at + "/service/".length);
  for (const m of text.matchAll(/crate::capabilities::(?:[a-z_][a-z0-9_]*::)+service::([A-Za-z0-9_:]+)/g)) {
    const segs = m[1].split("::").filter((s) => s !== "self" && s !== "super");
    let best = null;
    for (let i = 1; i <= segs.length; i += 1) {
      const cand = base + segs.slice(0, i).join("/");
      if (nodes.has(cand)) best = cand;
    }
    if (best) pick(best);
  }
  return out;
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
  const files = walk(root);
  const nodes = new Set();
  for (const f of files) {
    if (scopeOf(f) === "service") nodes.add(modulePathOf(f));
  }
  for (const f of files) {
    const scope = scopeOf(f);
    if (!scope) continue;
    const raw = fs.readFileSync(path.join(root, f), "utf8");
    const prod = mask(stripTestModules(raw));
    const cap = f.split("/")[2];

    if (scope !== "detail" && prod.includes("crate::kernel::detail")) {
      outsideDetailKernel.add(f + " -> crate::kernel::detail");
    }

    if (scope === "service") {
      if (/crate::capabilities::(?:[a-z_][a-z0-9_]*::)*detail\b/.test(prod)) {
        serviceToDetail.add(f + " -> ::detail");
      }
      if (prod.includes("crate::kernel::detail")) serviceToDetail.add(f + " -> crate::kernel::detail");
      for (const call of IO_CALLS) {
        if (prod.includes(call)) serviceIo.add(f + " -> " + call);
      }
      const withoutPort = prod.replace(/\b(?:self\s*\.\s*)?probe\s*\.\s*canonicalize\s*\(/g, "");
      if (CANONICALIZE.test(withoutPort)) serviceIo.add(f + " -> Path::canonicalize");
      const node = modulePathOf(f);
      serviceGraph[node] = siblingEdges(node, prod, nodes);
    }
  }
  for (const n of nodes) serviceGraph[n] = serviceGraph[n] || new Set();
  const cycles = tarjanScc(serviceGraph).map((c) => c.map((n) => "src/" + n + ".rs").join(" ↔ "));
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
