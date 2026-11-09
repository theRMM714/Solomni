/**
 * 能力内部结构门禁：三条硬判据 + 一条只报不拦的文件粒度回执。
 * 硬判据：① 同能力 service 不得引用本能力 detail 或 kernel::detail；② service 不得直接做 IO；
 *        ③ 同一能力的 service 方法族之间不得成环。豁免与快照在 tests/structure-baseline.json，条目过期即失败。
 * 回执：service / domain / detail / api.rs 超 800 行按行数列出，不改变成败——行数是气味，依赖才是规则；
 *      唯一硬线是新建文件超过 1200 行（新文件没有历史包袱，超线即说明职责没拆）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");

const IO_CALLS = ["std::fs", "std::process::Command", "ureq", "reqwest", "std::net"];
const SOFT_LIMIT = 800;
const HARD_LIMIT = 1200;
const HEADER_BUDGET = 120;

/** 能力内的实现分区：api / service / domain / detail；其余（mod.rs、guest.rs 这类）不归这些判据管。 */
function scopeOf(f) {
  const parts = f.split("/");
  if (parts[0] !== "src" || parts[1] !== "capabilities" || parts.length < 4) return null;
  const rest = parts.slice(3).join("/");
  if (rest === "api.rs") return "api";
  for (const name of ["service", "domain", "detail"]) {
    if (rest === name + ".rs" || rest.startsWith(name + "/")) return name;
  }
  return null;
}

/** 文件头的 //! 段落：超过预算说明"这个文件一句话说不清自己是什么"。 */
function headerSlot(text) {
  const lines = text.split(/\r?\n/);
  let body = "";
  for (const line of lines.slice(0, 40)) {
    if (!line.startsWith("//!")) {
      if (body) break;
      continue;
    }
    body += line.replace(/^\/\/!\s?/, "");
  }
  if (!body) return "缺";
  return body.replace(/\s+/g, "").length > HEADER_BUDGET ? "写不下" : "能";
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
  const serviceIo = new Set();
  const serviceGraph = {};
  const sizes = {};
  const notes = [];
  for (const f of walk(root)) {
    const scope = scopeOf(f);
    if (!scope) continue;
    const raw = fs.readFileSync(path.join(root, f), "utf8");
    const rawLines = raw.split(/\r?\n/);
    if (rawLines[rawLines.length - 1] === "") rawLines.pop();
    const lines = rawLines.length;
    sizes[f] = lines;
    const prod = stripTestModules(raw);
    const cap = f.split("/")[2];

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

    if (lines > SOFT_LIMIT) {
      const io = IO_CALLS.reduce((n, c) => n + prod.split(c).length - 1, 0);
      const impl = (prod.match(/^impl\b/gm) || []).length;
      notes.push({ file: f, lines, io, impl, header: headerSlot(raw) });
    }
  }
  notes.sort((a, b) => b.lines - a.lines);
  const cycles = tarjanScc(serviceGraph).map((c) => "capabilities/" + c.join(", "));
  return { serviceToDetail: [...serviceToDetail].sort(), serviceIo: [...serviceIo].sort(), cycles, sizes, notes };
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
  const notes = actual.notes.map((n) => {
    const known = base.bigFiles && typeof base.bigFiles[n.file] === "number" ? base.bigFiles[n.file] : null;
    const delta = known === null ? "新" : n.lines - known;
    const deltaText = delta === "新" ? "新文件" : delta > 0 ? "+" + delta : String(delta);
    return "[big] " + n.file + "  " + n.lines + " 行 (" + deltaText + ")  io=" + n.io + " impl=" + n.impl + "  目的槽=" + n.header;
  });

  // 新建文件超过硬线：只有"基线里没有这个文件"才算新建——存量永不为行数失败。
  for (const n of actual.notes) {
    const known = base.bigFiles && typeof base.bigFiles[n.file] === "number";
    if (!known && n.lines > HARD_LIMIT) {
      problems.push("文件粒度：新建文件超过 " + HARD_LIMIT + " 行（先拆职责，不要为过线机械拆文件）：" + n.file + "（" + n.lines + " 行）");
    }
  }

  const compare = (name, actualList, baselineList, hint) => {
    for (const a of actualList) if (!baselineList.includes(a)) problems.push("能力内部：" + hint + "：" + a);
    for (const b of baselineList) {
      if (!actualList.includes(b)) {
        problems.push("能力内部：基线豁免已过期，请从 " + rel(root, baselineFile) + " 的 " + name + " 删除：" + b);
      }
    }
  };
  compare("serviceToDetail", actual.serviceToDetail, base.serviceToDetail || [], "service 不得引用本能力 detail 或 kernel::detail（抽端口，别直连实现）");
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
  return { problems, notes };
}

module.exports = { internal, scan, LIMITS: { SOFT_LIMIT, HARD_LIMIT } };
