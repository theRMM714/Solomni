/**
 * 能力内部结构门禁：四条硬判据。文件粒度（行数、内联测试）由 structure/size.js 一处判，这里不管。
 * 硬判据：① 同能力 service 不得引用本能力 detail 或 kernel::detail；② service 不得直接做 IO；
 *        ③ 同一能力的 service 方法族之间不得成环；④ 能力内 detail 之外的文件不得引用 kernel::detail。
 * 覆盖 api / service / domain / detail 四个分区（api 含 api.rs 与 api/ 子目录）。
 * 豁免在 tests/structure-baseline.json，条目过期即失败。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");

const IO_CALLS = ["std::fs", "std::process::Command", "ureq", "reqwest", "std::net"];

/** 能力内的实现分区：api / service / domain / detail；其余（mod.rs、guest.rs 这类）不归这些判据管。 */
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
    const prod = stripTestModules(raw);
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
      const node = "capabilities/" + cap + "/service/" + f.split("/").pop();
      serviceGraph[node] = siblingEdges(f, prod);
    }
  }
  const cycles = tarjanScc(serviceGraph).map((c) => "capabilities/" + c.join(", "));
  return { serviceToDetail: [...serviceToDetail].sort(), outsideDetailKernel: [...outsideDetailKernel].sort(), serviceIo: [...serviceIo].sort(), cycles };
}

function sortCycles(list) {
  return list.map((s) => s.slice().sort()).sort((a, b) => a.join(",").localeCompare(b.join(",")));
}

function internal(root) {
  const baselineFile = path.join(root, "tests", "structure-baseline.json");
  const problems = [];
  let base = null;
  try {
    base = JSON.parse(fs.readFileSync(baselineFile, "utf8"));
  } catch (e) {
    return { problems: ["能力内部：基线不可读或不是合法 JSON：" + rel(root, baselineFile) + "（" + e.message + "）"], notes: [] };
  }
  const actual = scan(root);

  const compare = (name, actualList, baselineList, hint) => {
    for (const a of actualList) if (!baselineList.includes(a)) problems.push("能力内部：" + hint + "：" + a);
    for (const b of baselineList) {
      if (!actualList.includes(b)) {
        problems.push("能力内部：基线豁免已过期，请从 " + rel(root, baselineFile) + " 的 " + name + " 删除：" + b);
      }
    }
  };
  compare("serviceToDetail", actual.serviceToDetail, base.serviceToDetail || [], "service 不得引用本能力 detail 或 kernel::detail（抽端口，别直连实现）");
  compare("outsideDetailKernel", actual.outsideDetailKernel, base.outsideDetailKernel || [], "detail 之外的能力文件不得引用 kernel::detail（由组合根注入端口）");
  compare("serviceIo", actual.serviceIo, base.serviceIo || [], "service 不得直接做 IO（fs / Command / 网络）；经端口或 detail");

  const got = sortCycles(actual.cycles);
  const want = sortCycles(base.siblingCycles || []);
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    problems.push(
      "能力内部：同能力方法族之间的环与基线不一致（拆完后同步更新 " + rel(root, baselineFile) + " 的 siblingCycles）：" +
      "\n    实际：" + (got.length ? got.map((s) => "[" + s.length + "] " + s.join(", ")).join("；") : "（无）") +
      "\n    基线：" + (want.length ? want.map((s) => "[" + s.length + "] " + s.join(", ")).join("；") : "（无）")
    );
  }
  return { problems, notes: [] };
}

module.exports = { internal, scan };
