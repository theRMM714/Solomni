/**
 * 结构审查·结构图：每次跑门禁把当前架构画成一张离线 HTML（仓库根 Structure.html）。
 * 目的：把「分层 / 依赖方向 / 出口 / 文件头四槽」变成一眼可见的东西——节点与边的颜色只表示门禁会拦什么，不自己判好坏。
 * 管：一张全景 + 四个视图。全景只画四层与层间聚合数（entry 装配 → presentation 屏 → units 单元 → kernel 机制）；
 *     各视图调各自门禁的判据收事实（layers / kernel-mechanisms / entry-tiers / presentation-inbound 的层表与边，
 *     以及 dependency / capability / values / size / comments / unit-map 的问题清单），按各自的登记表分层排布；
 *     点节点下钻到文件级依赖，点文件看四槽与命中问题；产物每次覆盖，不进库（.gitignore 已忽略）。
 * 不管：不新增判据、不判红（永远 pass）、不改任何源码；除 Structure.html 之外不写文件。
 * 联动：layers.js、kernel-mechanisms.js、entry-tiers.js、presentation-inbound.js 的边与层表（四个视图各用各的权威，
 *       图里不重算判据）、run-hygiene.js 的 headerOf（四槽唯一解析器）、tests/gates/registry.js 的登记。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const L = require("./layers.js");
const U = require("./unitpath.js");
const K = require("./kernel-mechanisms.js");
const E = require("./entry-tiers.js");
const P = require("./presentation-inbound.js");
const presentation = require("../../lib/presentation.js");
const dependency = require("./dependency.js");
const capability = require("./capability.js");
const values = require("./values.js");
const size = require("./size.js");
const comments = require("./comments.js");
const units = require("./map.js");
const { mask } = require("../../lib/rust-mask.js");

const OUT = "Structure.html";
const GAP_X = 14;
const BAND_GAP = 26;
const NODE_W = 150;
const NODE_H = 44;

/** 视图顺序；tab 文案与 JS 里的切换逻辑共用这一份。 */
const VIEWS = [
  { key: "overview", tab: "全景" },
  { key: "caps", tab: "单元环" },
  { key: "mech", tab: "kernel 机制" },
  { key: "entry", tab: "入口" },
  { key: "pres", tab: "屏面" },
];

/** 每个视图右栏的说明；改文案只改这里。 */
const HINTS = {
  overview:
    "<h2>怎么看 · 全景</h2><p class=hint>四层从上到下：entry 装配 → presentation 屏 → units 单元 → kernel 机制。带内数字是聚合事实，细边在各自的视图里。点任一带进入该视图；红边是门禁会拦的向上/同层依赖。</p>",
  caps:
    "<h2>怎么看 · 单元依赖</h2><p class=hint>矩阵的行是引用方、列是被引用方，格内数字是引用处数，颜色越深越多；红框是门禁会拦的向上依赖或同环逆序（当前为 0）。行按环分块，越靠上环号越大；右侧「出」是该单元对其它单元的总引用处数。悬停一格只看它的一行一列；点格或点名字进单元详情，再点文件看目的 / 管 / 不管 / 联动。下方按引用处数列出强边，供直接读「谁最依赖谁」。</p>",
  mech:
    "<h2>怎么看 · kernel 机制</h2><p class=hint>一个机制管一种机制，边是机制之间的引用；严格向下，同层与向上都是红色。层号以 docs/kernel/mechanisms.yaml 为准（它是按依赖重算的事实缓存）。虚线框是扇出前三。</p>",
  entry:
    "<h2>怎么看 · 入口分段</h2><p class=hint>启动流程四段：值层 → 判定层 → 装配层 → 编排层。边是模块之间的引用，同段互依赖与向上依赖都是红色。段号以 docs/entry/layers.yaml 为准；main 只串流程。</p>",
  pres:
    "<h2>怎么看 · 屏面入站面</h2><p class=hint>左边一屏（presentation 里的文件），右边它取的单元。清单在 docs/presentation/inbound.yaml，源码与清单双向比对；红边是清单外的取用或同层互依赖。点一屏看它的单元与四槽。</p>",
};

/** 仓库相对路径（正斜杠）。 */
const relOf = (root, f) => f.slice(root.length + 1).replace(/\\/g, "/");

/** 路径所属单元；不在单元目录下返回空串。 */
function capOf(rel) {
  const m = /^src\/units\/(?:pure\/)?([a-z_][a-z0-9_]*)\//.exec(rel);
  return m ? m[1] : "";
}

/** 文案里出现的所有 .rs 路径（判据的锚点就是这个形状），用于把问题挂到节点上。 */
function pathsIn(text) {
  return String(text).match(/[A-Za-z0-9_./-]+\.rs(?::\d+)?/g) || [];
}

/** 去重：同一条问题被多个判据报出来时只留一份。 */
function dedupe(list) {
  const seen = new Set();
  const out = [];
  for (const p of list) {
    const k = p.gate + "\u0000" + p.text;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(p);
  }
  return out;
}

/** 四类门禁的问题清单：结构图只搬运，不产生判断。 */
function judgeProblems(root) {
  const cap = [];
  const push = (gate, list) => {
    for (const t of list || []) cap.push({ gate: gate, text: String(t) });
  };
  push("layers", L.layers(root).problems);
  push("dependency", dependency.direction(root));
  push("capability", capability.internal(root).problems);
  push("values", values.values(root).problems);
  push("size", size.fileSize(root).problems);
  push("comments", comments.contract(root).problems);
  push("unit-map", units.unitMap(root));
  const of = (gate, list) => (list || []).map((t) => ({ gate: gate, text: String(t) }));
  const mech = of("kernel-mechanisms", K.kernelMechanisms(root).problems);
  const entry = of("entry-tiers", E.entryTiers(root).problems);
  const pres = of("presentation-inbound", P.presentationInbound(root).problems);
  return {
    cap: dedupe(cap),
    mech: dedupe(mech),
    entry: dedupe(entry),
    pres: dedupe(pres),
    all: dedupe(cap.concat(mech, entry, pres)),
  };
}

/** 一条问题挂到哪些名字上：文案里出现的名字都算命中（一条边的问题会同时挂两端）。 */
function attachByNames(problems, names) {
  const out = new Map();
  const sorted = names.slice().sort((a, b) => b.length - a.length);
  const hit = (text, n) => {
    const re = new RegExp("(^|[^a-z0-9_/])" + n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "($|[^a-z0-9_/])");
    return re.test(text);
  };
  for (const p of problems) {
    let any = false;
    for (const n of sorted) {
      if (!hit(p.text, n)) continue;
      if (!out.has(n)) out.set(n, []);
      out.get(n).push(p.text);
      any = true;
    }
    if (!any) {
      if (!out.has("")) out.set("", []);
      out.get("").push(p.text);
    }
  }
  return out;
}

/** 问题文案里点到某个文件的，挂到文件级（点文件时能看到）。 */
function attachByFiles(problems) {
  const out = new Map();
  for (const p of problems) {
    for (const f of pathsIn(p.text).map((x) => x.replace(/:\d+$/, ""))) {
      if (!out.has(f)) out.set(f, []);
      out.get(f).push(p.text);
    }
  }
  return out;
}

/** 文件级事实：行数（size.scan 的口径）、角色、软线、四槽（稍后统一填）、命中问题。 */
function fileRows(rows, rels, fileHit) {
  return rels.map((p) => {
    const row = rows.get(p) || { prod: 0, total: 0, role: "impl" };
    const soft = row.role === "impl" ? row.prod > size.LIMITS.impl.prodSoft : row.total > size.LIMITS.support.soft;
    return { path: p, role: row.role, prod: row.prod, total: row.total, soft: soft, head: {}, issues: fileHit.get(p) || [] };
  });
}

/** 同组内的文件依赖（下钻用）：只认组内文件，组外引用画到节点级的边上。 */
function edgesWithin(root, files, prefixes) {
  const known = new Set(files);
  const out = new Map();
  const add = (from, to) => {
    if (!to || to === from || !known.has(to)) return;
    const k = from + "\u0000" + to;
    const hit = out.get(k);
    if (hit) hit.refs++;
    else out.set(k, { from: from, to: to, refs: 1 });
  };
  const resolveTail = (bases, tail) => {
    for (let n = tail.length; n >= 1; n--) {
      for (const b of bases) {
        const stem = b + "/" + tail.slice(0, n).join("/");
        if (known.has(stem + ".rs")) return stem + ".rs";
        if (known.has(stem + "/mod.rs")) return stem + "/mod.rs";
      }
    }
    for (const b of bases) {
      if (known.has(b + ".rs")) return b + ".rs";
      if (known.has(b + "/mod.rs")) return b + "/mod.rs";
    }
    return "";
  };
  const resolve = (from, segs) => {
    if (segs[0] === "super" || segs[0] === "self") {
      let rest = segs.slice(1);
      let base = path.posix.dirname(from);
      while (rest[0] === "super") {
        rest = rest.slice(1);
        base = path.posix.dirname(base);
      }
      if (rest[0] === "self") rest = rest.slice(1);
      const bases = segs[0] === "self" ? [from.replace(/\.rs$/, ""), base] : [base];
      return resolveTail(bases, rest);
    }
    for (const raw of prefixes) {
      const pre = raw[0] === "crate" ? raw.slice(1) : raw;
      if (segs.slice(0, pre.length).join("::") !== pre.join("::")) continue;
      const tail = segs.slice(pre.length).filter((s) => s !== "self" && s !== "super");
      return tail.length ? resolveTail(["src/" + pre.join("/")], tail) : "";
    }
    return "";
  };
  for (const from of [...known].sort()) {
    const text = dependency
      .stripTestModules(fs.readFileSync(path.join(root, from), "utf8"))
      .split(/\r?\n/)
      .filter((l) => !l.trim().startsWith("//"))
      .join("\n");
    for (const m of text.matchAll(/\bcrate(?:::[a-z_][a-z0-9_]*)+/g)) add(from, resolve(from, m[0].split("::").slice(1)));
    for (const m of text.matchAll(/\bsuper(?:::[a-z_][a-z0-9_]*)+/g)) add(from, resolve(from, m[0].split("::")));
    for (const m of text.matchAll(/\bself(?:::[a-z_][a-z0-9_]*)+/g)) add(from, resolve(from, m[0].split("::")));
    for (const m of text.matchAll(/(?<!:)ENTRY_PREFIX(?:::[a-z_][a-z0-9_]*)+/g)) add(from, resolve(from, m[0].split("::")));
  }
  return [...out.values()].sort((a, b) => (a.from < b.from ? -1 : a.from > b.from ? 1 : a.to < b.to ? -1 : 1));
}

/** 单元环：环号、rank 与边取 layers.js，问题挂到单元与文件上。 */
function capView(root, rows, problems) {
  const { tiers } = L.parseLayers(root);
  const dirs = L.dirsOf(root);
  const capEdges = L.edgesOf(root, dirs);
  const fileHit = attachByFiles(problems);
  const byCap = new Map();
  for (const p of problems) {
    const hit = pathsIn(p.text).map((x) => x.replace(/:\d+$/, "")).find((x) => x.startsWith("src/units/"));
    const cap = hit ? capOf(hit) : "";
    if (!cap) continue;
    if (!byCap.has(cap)) byCap.set(cap, []);
    byCap.get(cap).push(p.text);
  }
  const nodes = dirs.map((name) => {
    const layer = tiers.has(name) ? tiers.get(name) : 0;
    const rels = L.rsFilesOf(L.unitDir(root, name)).map((f) => relOf(root, f)).sort();
    const dir = relOf(root, L.unitDir(root, name));
    return { name: name, layer: layer, dir: dir, out: 0, inn: 0, files: fileRows(rows, rels, fileHit), issues: byCap.get(name) || [] };
  });
  const byName = new Map(nodes.map((c) => [c.name, c]));
  const ranks = L.ranksOf(tiers, capEdges);
  const edges = [];
  for (const [key, info] of [...capEdges].sort()) {
    const [from, to] = key.split(">");
    edges.push({ from: from, to: to, refs: info.refs, file: relOf(root, info.file), violation: !!L.edgeProblem(tiers, ranks.rank, from, to) });
    if (byName.has(from)) byName.get(from).out++;
    if (byName.has(to)) byName.get(to).inn++;
  }
  const fe = {};
  for (const node of nodes) {
    fe[node.name] = edgesWithin(root, node.files.map((f) => f.path), [U.unitPrefix(root, node.name)]);
  }
  return { nodes: nodes, edges: edges, fe: fe };
}

/** kernel 机制层：层号与边取 kernel-mechanisms.js，问题挂到机制与文件上。 */
function mechView(root, rows, problems) {
  const { tiers } = K.parseMechanisms(root);
  const mechs = K.mechanismsOf(root);
  const names = [...tiers.keys()].concat([...mechs.keys()]);
  const byName = attachByNames(problems, names);
  const fileHit = attachByFiles(problems);
  const edgeMap = K.edgesOf(root, mechs);
  const nodes = [...mechs.keys()].sort().map((name) => {
    const rels = mechs.get(name).map((f) => relOf(root, f)).sort();
    return { name: name, layer: tiers.has(name) ? tiers.get(name) : 0, out: 0, inn: 0, files: fileRows(rows, rels, fileHit), issues: byName.get(name) || [] };
  });
  const byNode = new Map(nodes.map((n) => [n.name, n]));
  const edges = [];
  for (const [key, info] of [...edgeMap].sort()) {
    const [from, to] = key.split(">");
    const a = tiers.get(from);
    const b = tiers.get(to);
    edges.push({ from: from, to: to, refs: info.refs, file: relOf(root, info.file), violation: b >= a });
    if (byNode.has(from)) byNode.get(from).out++;
    if (byNode.has(to)) byNode.get(to).inn++;
  }
  const fe = {};
  for (const node of nodes) {
    const [area, mech] = node.name.split("/");
    fe[node.name] = edgesWithin(root, node.files.map((f) => f.path), [["crate", "kernel", area, mech]]);
  }
  return { nodes: nodes, edges: edges, fe: fe, layers: Math.max(0, ...tiers.values()) + 1 };
}

/** 入口分段：段号与边取 entry-tiers.js；main 单列。 */
function entryView(root, rows, problems) {
  const { tiers } = E.parseEntryTiers(root);
  const mods = E.modulesOf(root);
  const byName = attachByNames(problems, [...tiers.keys()]);
  const fileHit = attachByFiles(problems);
  const edgeMap = E.edgesOf(root, mods);
  const nodes = [...mods.keys()].sort().map((name) => {
    const rels = mods.get(name).map((f) => relOf(root, f)).sort();
    return { name: name, layer: tiers.has(name) ? tiers.get(name) : 0, out: 0, inn: 0, files: fileRows(rows, rels, fileHit), issues: byName.get(name) || [] };
  });
  const byNode = new Map(nodes.map((n) => [n.name, n]));
  const edges = [];
  for (const [key, info] of [...edgeMap].sort()) {
    const [from, to] = key.split(">");
    const a = tiers.get(from);
    const b = tiers.get(to);
    edges.push({ from: from, to: to, refs: info.refs, file: relOf(root, info.file), violation: b >= a });
    if (byNode.has(from)) byNode.get(from).out++;
    if (byNode.has(to)) byNode.get(to).inn++;
  }
  const fe = {};
  for (const node of nodes) {
    const pre = node.name === "main" ? [["crate", "entry"], ["entry"]] : [["crate", "entry", node.name]];
    fe[node.name] = edgesWithin(root, node.files.map((f) => f.path), pre);
  }
  return { nodes: nodes, edges: edges, fe: fe, segments: Math.max(0, ...tiers.values()) + 1 };
}

/** 屏面入站面：每屏取哪些单元，清单取 presentation-inbound.js 的解析器（唯一权威）。 */
function presView(root, rows, problems) {
  const file = path.join(root, "docs", "presentation", "inbound.yaml");
  const parsed = fs.existsSync(file) ? presentation.parseInboundText(fs.readFileSync(file, "utf8")) : { faces: new Map(), problems: [] };
  const fileHit = attachByFiles(problems);
  const screens = [...parsed.faces.keys()].sort().map((f) => {
    const list = parsed.faces.get(f).slice().sort();
    return {
      name: f.slice("src/presentation/".length),
      file: f,
      layer: 0,
      units: list,
      files: fileRows(rows, [f], fileHit),
      issues: fileHit.get(f) || [],
    };
  });
  const unitNames = [...new Set(screens.reduce((acc, s) => acc.concat(s.units), []))].sort();
  const edges = [];
  for (const s of screens) {
    for (const u of s.units) {
      const violation = problems.some((p) => p.text.indexOf(s.file) >= 0 && p.text.indexOf(u) >= 0);
      edges.push({ from: s.file, to: u, refs: 1, violation: violation });
    }
  }
  return { screens: screens, units: unitNames, edges: edges };
}

/** 跨层聚合：全景只报数，细边在各视图里。全部只读源码与清单，不产生判据。 */
function crossCounts(root, capsView, mechViewData, entryViewData, pres) {
  const pairs = { entryCap: new Set(), entryPres: new Set(), entryKernel: new Set(), capKernel: new Set() };
  const mods = E.modulesOf(root);
  for (const [mod, files] of mods) {
    for (const f of files) {
      const text = mask(dependency.stripTestModules(fs.readFileSync(f, "utf8")));
      for (const m of text.matchAll(/crate::units::([a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)*)/g)) pairs.entryCap.add(mod + ">" + P.unitOf(m[1]));
      for (const m of text.matchAll(/crate::presentation(?:::[a-z_][a-z0-9_]*)+/g)) pairs.entryPres.add(mod + ">" + m[0]);
      if (/crate::kernel::api/.test(text)) pairs.entryKernel.add(mod);
    }
  }
  for (const cap of capsView.nodes) {
    for (const f of cap.files) {
      const text = mask(dependency.stripTestModules(fs.readFileSync(path.join(root, f.path), "utf8")));
      for (const m of text.matchAll(/crate::kernel::api::([a-z_][a-z0-9_]*)/g)) pairs.capKernel.add(cap.name + ">" + m[1]);
    }
  }
  const faces = new Set([...pairs.capKernel].map((k) => k.split(">")[1]));
  return {
    entryCap: pairs.entryCap.size,
    entryPres: pairs.entryPres.size,
    entryKernel: pairs.entryKernel.size,
    capKernel: pairs.capKernel.size,
    faces: faces.size,
    screenEdges: pres.edges.length,
    screenKernel: pres.screens.filter((s) => s.units.indexOf("kernel") >= 0).length,
  };
}

/** 收事实：四层节点、边、文件、问题。全部排序，保证同输入同输出。 */
function collect(root) {
  const rows = new Map(size.scan(root).map((r) => [r.file, r]));
  const problems = judgeProblems(root);
  const caps = capView(root, rows, problems.cap);
  const mech = mechView(root, rows, problems.mech);
  const entry = entryView(root, rows, problems.entry);
  const pres = presView(root, rows, problems.pres);

  const files = [].concat(
    caps.nodes.reduce((acc, c) => acc.concat(c.files), []),
    mech.nodes.reduce((acc, c) => acc.concat(c.files), []),
    entry.nodes.reduce((acc, c) => acc.concat(c.files), []),
    pres.screens.reduce((acc, c) => acc.concat(c.files), []),
  );
  const heads = require(path.join(root, "run-hygiene.js")).headers(files.map((f) => f.path));
  for (const f of files) f.head = heads[f.path] || {};

  const wide = caps.nodes.slice().sort((a, b) => b.out - a.out || a.name.localeCompare(b.name)).slice(0, 3).map((c) => c.name);
  const wideMech = mech.nodes.slice().sort((a, b) => b.out - a.out || a.name.localeCompare(b.name)).slice(0, 3).map((c) => c.name);
  const wideEntry = entry.nodes.slice().sort((a, b) => b.out - a.out || a.name.localeCompare(b.name)).slice(0, 3).map((c) => c.name);
  const cross = crossCounts(root, caps, mech, entry, pres);

  return {
    meta: {
      wide: wide,
      wideMech: wideMech,
      wideEntry: wideEntry,
      caps: caps.nodes.length,
      edges: caps.edges.length,
      files: caps.nodes.reduce((n, c) => n + c.files.length, 0),
      band: Math.max(...caps.nodes.map((c) => c.layer), 0),
      mech: mech.nodes.length,
      mechLayers: mech.layers,
      mechEdges: mech.edges.length,
      entry: entry.nodes.length,
      entrySegments: entry.segments,
      entryEdges: entry.edges.length,
      screens: pres.screens.length,
      unitCount: pres.units.length,
      screenFiles: pres.screens.length,
      issues: problems.all.length,
      cross: cross,
    },
    caps: caps.nodes,
    edges: caps.edges,
    fe: caps.fe,
    mech: { nodes: mech.nodes, edges: mech.edges, fe: mech.fe, wide: wideMech },
    entry: { nodes: entry.nodes, edges: entry.edges, fe: entry.fe, wide: wideEntry },
    pres: { screens: pres.screens, units: pres.units, edges: pres.edges },
    problems: { all: problems.all.length, mech: problems.mech.length, entry: problems.entry.length, pres: problems.pres.length },
    hints: HINTS,
  };
}
module.exports = { collect };

/** HTML / SVG 文本转义。 */
function esc(s) {
  return String(s == null ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** 按层排布：层号大的在上（编排层在顶），同层一行居中。nodeW/nodeH 由各视图给。 */
function layout(nodes, nodeW, nodeH) {
  const layers = [...new Set(nodes.map((c) => c.layer))].sort((a, b) => b - a);
  const widest = Math.max(1, ...layers.map((l) => nodes.filter((c) => c.layer === l).length));
  const width = Math.max(820, widest * (nodeW + GAP_X) + 40);
  const pos = new Map();
  const bands = [];
  let y = 14;
  for (const layer of layers) {
    const row = nodes.filter((c) => c.layer === layer).sort((a, b) => a.name.localeCompare(b.name));
    const rowW = row.length * (nodeW + GAP_X) - GAP_X;
    const x0 = width > 1240 ? 20 : Math.max(20, (width - rowW) / 2);
    const top = y + 20;
    row.forEach((c, i) => pos.set(c.name, { x: x0 + i * (nodeW + GAP_X), y: top, layer: layer }));
    const h = 20 + nodeH + 16;
    bands.push({ layer: layer, y: y, h: h, n: row.length });
    y += h + BAND_GAP;
  }
  return { pos: pos, bands: bands, width: width, height: y };
}

/** 分层视图（单元 / 机制 / 入口共用）：带内铺层，边按门禁口径着色（越界红、常规灰）。 */
function bandSvg(data, opts) {
  const nodeW = opts.nodeW;
  const nodeH = opts.nodeH;
  const wide = opts.wide || [];
  const label = opts.bandLabel;
  const aria = opts.aria;
  const lay = layout(data.nodes, nodeW, nodeH);
  const out = [];
  out.push('<svg class="' + (lay.width > 1240 ? "wide" : "") + '" viewBox="0 0 ' + lay.width + " " + lay.height + '" width="' + lay.width + '" height="' + lay.height + '" role="img" aria-label="' + esc(aria) + '">');
  out.push('<defs><marker id="ar' + opts.id + '" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#94a3b8"/></marker><marker id="arv' + opts.id + '" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#dc2626"/></marker></defs>');
  for (const b of lay.bands) {
    out.push('<rect x="0" y="' + b.y + '" width="' + lay.width + '" height="' + b.h + '" fill="' + (b.layer % 2 ? "#f8fafc" : "#ffffff") + '"/>');
    out.push('<text x="14" y="' + (b.y + 15) + '" class="band">' + esc(label(b.layer)) + (b.n ? " · " + b.n + " 个" : " · 空") + "</text>");
  }
  const violationOf = new Set();
  for (const e of data.edges) if (e.violation) violationOf.add(e.from), violationOf.add(e.to);
  for (const e of data.edges) {
    const a = lay.pos.get(e.from);
    const b = lay.pos.get(e.to);
    if (!a || !b) continue;
    const down = b.y > a.y;
    const x1 = a.x + nodeW / 2;
    const y1 = down ? a.y + nodeH : a.y;
    const x2 = b.x + nodeW / 2;
    const y2 = down ? b.y : b.y + nodeH;
    const dy = y2 - y1;
    const c1 = y1 + dy * 0.45;
    const c2 = y2 - dy * 0.45;
    const w = (0.7 + Math.min(e.refs, 12) * 0.14).toFixed(2);
    out.push('<path class="edge' + (e.violation ? " bad" : "") + '" data-from="' + esc(e.from) + '" data-to="' + esc(e.to) + '" d="M' + x1 + "," + y1 + " C" + x1 + "," + c1 + " " + x2 + "," + c2 + " " + x2 + "," + y2 + '" fill="none" stroke="' + (e.violation ? "#dc2626" : "#94a3b8") + '" stroke-width="' + w + '" stroke-opacity="' + (e.violation ? "0.95" : "0.5") + '" marker-end="url(#' + (e.violation ? "arv" : "ar") + opts.id + ')"><title>' + esc(e.from + " → " + e.to + "：" + e.refs + " 处引用" + (e.violation ? "（越界）" : "")) + "</title></path>");
  }
  for (const c of data.nodes) {
    const p = lay.pos.get(c.name);
    const bad = c.issues.length > 0 || violationOf.has(c.name);
    const isWide = wide.indexOf(c.name) >= 0;
    const stroke = bad ? "#dc2626" : isWide ? "#f59e0b" : "#cbd5e1";
    const dash = !bad && isWide ? ' stroke-dasharray="5 4"' : "";
    const cut2 = c.name.indexOf("/");
    const area = cut2 > 0 ? c.name.slice(0, cut2) : "";
    const base = cut2 > 0 ? c.name.slice(cut2 + 1) : c.name;
    const short = base.length > 21 ? base.slice(0, 20) + "…" : base;
    const sub = "出 " + c.out + " · 入 " + c.inn + " · " + c.files.length + " 文件";
    out.push('<g class="node" data-view="' + esc(opts.view) + '" data-name="' + esc(c.name) + '" transform="translate(' + p.x + "," + p.y + ')"><title>' + esc(c.name + " · " + label(c.layer) + " · " + sub + (c.issues.length ? " · " + c.issues.length + " 处命中" : "")) + "</title>");
    out.push('<rect width="' + nodeW + '" height="' + nodeH + '" rx="9" fill="#ffffff" stroke="' + stroke + '" stroke-width="' + (bad ? 2 : 1.2) + '"' + dash + "/>");
    if (area) {
      out.push('<text x="12" y="13" class="sub" style="font-size:9px">' + esc(area) + "</text>");
      out.push('<text x="12" y="28" class="nm" style="font-size:12px">' + esc(short) + "</text>");
      out.push('<text x="12" y="40" class="sub" style="font-size:9.5px">' + esc(sub) + "</text>");
    } else {
      out.push('<text x="12" y="19" class="nm">' + esc(short) + "</text>");
      out.push('<text x="12" y="34" class="sub">' + esc(sub) + "</text>");
    }
    if (c.issues.length) out.push('<circle cx="' + (nodeW - 13) + '" cy="13" r="8" fill="#dc2626"/><text x="' + (nodeW - 13) + '" y="17" class="badge" text-anchor="middle">' + (c.issues.length > 9 ? "9+" : c.issues.length) + "</text>");
    out.push("</g>");
  }
  out.push("</svg>");
  return out.join("");
}

/** 屏面视图：左边一屏，右边它取的单元。 */
function presSvg(pres, wide) {
  const SW = 300;
  const SH = 40;
  const SGAP = 12;
  const UW = 210;
  const UH = 28;
  const UGAP = 9;
  const LEFT = 20;
  const RIGHT = LEFT + SW + 130;
  const top = 26;
  const h1 = pres.screens.length * (SH + SGAP);
  const h2 = pres.units.length * (UH + UGAP);
  const W = RIGHT + UW + 30;
  const H = Math.max(h1, h2) + top + 30;
  const out = [];
  out.push('<svg class="' + (W > 1240 ? "wide" : "") + '" viewBox="0 0 ' + W + " " + H + '" width="' + W + '" height="' + H + '" role="img" aria-label="屏面入站面">');
  out.push('<defs><marker id="par" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#94a3b8"/></marker></defs>');
  out.push('<text x="' + LEFT + '" y="16" class="band">屏（src/presentation）</text><text x="' + RIGHT + '" y="16" class="band">入站面单元（inbound.yaml）</text>');
  const SP = new Map();
  const UP = new Map();
  pres.screens.forEach((s, i) => SP.set(s.file, { x: LEFT, y: top + i * (SH + SGAP) }));
  pres.units.forEach((u, i) => UP.set(u, { x: RIGHT, y: top + i * (UH + UGAP) }));
  for (const e of pres.edges) {
    const a = SP.get(e.from);
    const b = UP.get(e.to);
    if (!a || !b) continue;
    const x1 = a.x + SW;
    const y1 = a.y + SH / 2;
    const x2 = b.x;
    const y2 = b.y + UH / 2;
    const dx = x2 - x1;
    out.push('<path class="edge' + (e.violation ? " bad" : "") + '" d="M' + x1 + "," + y1 + " C" + (x1 + dx * 0.4) + "," + y1 + " " + (x2 - dx * 0.4) + "," + y2 + " " + x2 + "," + y2 + '" fill="none" stroke="' + (e.violation ? "#dc2626" : "#94a3b8") + '" stroke-width="1" stroke-opacity="' + (e.violation ? "0.95" : "0.45") + '" marker-end="url(#par)"><title>' + esc(e.from + " → " + e.to) + "</title></path>");
  }
  pres.screens.forEach((s) => {
    const p = SP.get(s.file);
    const bad = s.issues.length > 0;
    out.push('<g class="node" data-view="pres" data-name="' + esc(s.file) + '" transform="translate(' + p.x + "," + p.y + ')"><title>' + esc(s.file + " · 取 " + s.units.length + " 个单元") + "</title>");
    out.push('<rect width="' + SW + '" height="' + SH + '" rx="8" fill="#ffffff" stroke="' + (bad ? "#dc2626" : "#cbd5e1") + '" stroke-width="' + (bad ? 2 : 1.2) + '"/>');
    out.push('<text x="10" y="17" class="nm">' + esc(s.name) + "</text>");
    out.push('<text x="10" y="31" class="sub">' + esc("取 " + s.units.length + " 个单元" + (s.issues.length ? " · " + s.issues.length + " 处命中" : "")) + "</text>");
    out.push("</g>");
  });
  pres.units.forEach((u) => {
    const p = UP.get(u);
    out.push('<g transform="translate(' + p.x + "," + p.y + ')"><title>' + esc(u) + "</title>");
    out.push('<rect width="' + UW + '" height="' + UH + '" rx="6" fill="#f8fafc" stroke="#cbd5e1"/>');
    out.push('<text x="9" y="18" class="sub" style="font-size:11px;fill:#0f172a">' + esc(u) + "</text>");
    out.push("</g>");
  });
  out.push("</svg>");
  return out.join("");
}

/** 单元依赖矩阵：行=引用方，列=被引用方，格内数字=引用处数（颜色按它加深），红框=门禁会拦的边。
 * 33 个单元、130 多条边的曲线图看不出谁依赖谁；矩阵把「谁 → 谁、几处」摊成可数的格子。
 * 只留有跨单元出边/入边的单元作图，其余列成一笔；下方再按引用处数排强边清单。只读 layers.js 的事实。 */
function capsSvg(nodes, edges, wide) {
  const CW = 26;
  const RH = 26;
  const LAB = 128;
  const TOP = 126;
  const BANDGAP = 10;
  const byName = new Map(nodes.map((c) => [c.name, c]));
  const byKey = new Map();
  for (const e of edges) byKey.set(e.from + "\u0000" + e.to, e);
  let maxRefs = 1;
  for (const e of edges) maxRefs = Math.max(maxRefs, e.refs);
  const outRefs = new Map();
  const inRefs = new Map();
  for (const e of edges) {
    outRefs.set(e.from, (outRefs.get(e.from) || 0) + e.refs);
    inRefs.set(e.to, (inRefs.get(e.to) || 0) + e.refs);
  }
  const sorter = (a, b) => b.layer - a.layer || a.name.localeCompare(b.name);
  const rows = nodes.filter((c) => (outRefs.get(c.name) || 0) > 0).sort(sorter);
  const cols = nodes.filter((c) => (inRefs.get(c.name) || 0) > 0).sort(sorter);
  const nr = rows.length;
  const nc = cols.length;
  const W = LAB + nc * CW + 60;
  const yOf = [];
  let y = TOP;
  for (let i = 0; i < nr; i++) {
    if (i && rows[i].layer !== rows[i - 1].layer) y += BANDGAP;
    yOf.push(y);
    y += RH;
  }
  const H = y + 30;
  const xOf = (i) => LAB + i * CW;
  const ringName = (l) => (l === 0 ? "环 0" : "环 " + l);
  const ringFull = (l) => (l === 0 ? "环 0 地基" : l === 1 ? "环 1 纯（无 IO）" : l === 2 ? "环 2 状态与机制" : "环 3 用例");
  const out = [];
  out.push('<div class="caps">');
  const rowBands = [];
  for (let i = 0; i < nr; i++) {
    const last = rowBands[rowBands.length - 1];
    if (last && last.layer === rows[i].layer) last.to = i;
    else rowBands.push({ layer: rows[i].layer, from: i, to: i });
  }
  const sums = { "3>2": [0, 0], "3>1": [0, 0], "2>1": [0, 0] };
  for (const e of edges) {
    const a = byName.get(e.from);
    const b = byName.get(e.to);
    if (!a || !b) continue;
    const k = a.layer + ">" + b.layer;
    if (sums[k]) {
      sums[k][0]++;
      sums[k][1] += e.refs;
    }
  }
  out.push('<svg class="matrix" viewBox="0 0 ' + W + " " + H + '" width="' + W + '" height="' + H + '" role="img" aria-label="单元依赖矩阵">');
  for (const b of rowBands) {
    out.push('<rect x="' + LAB + '" y="' + (yOf[b.from] - 5) + '" width="' + nc * CW + '" height="' + ((b.to - b.from + 1) * RH + 10) + '" fill="' + (b.layer === 3 ? "#f8fafc" : b.layer === 2 ? "#ffffff" : "#f1f5f9") + '"/>');
    out.push('<text x="6" y="' + (yOf[b.from] + 15) + '" class="band rband"><title>' + esc(ringFull(b.layer)) + " · " + (b.to - b.from + 1) + ' 个有出边的单元</title>' + esc(ringName(b.layer)) + "</text>");
  }
  // 列按环分组：组间一条细竖线，读列头时能看出它在哪一层。
  for (let j = 1; j < nc; j++) {
    if (cols[j].layer === cols[j - 1].layer) continue;
    out.push('<line x1="' + xOf(j) + '" y1="' + (TOP - 4) + '" x2="' + xOf(j) + '" y2="' + (y - 4) + '" stroke="#cbd5e1" stroke-dasharray="2 3"/>');
  }
  for (let i = 0; i < nr; i++) {
    out.push('<line x1="' + LAB + '" y1="' + (yOf[i] + RH) + '" x2="' + (LAB + nc * CW) + '" y2="' + (yOf[i] + RH) + '" stroke="#eef2f7"/>');
  }
  for (let i = 0; i < nr; i++) {
    const from = rows[i];
    for (let j = 0; j < nc; j++) {
      const to = cols[j];
      const e = byKey.get(from.name + "\u0000" + to.name);
      if (!e) continue;
      const a = 0.2 + 0.55 * Math.min(1, Math.log2(1 + e.refs) / Math.log2(1 + maxRefs));
      const cx = xOf(j);
      const cy = yOf[i];
      out.push('<g class="mcell" data-from="' + esc(from.name) + '" data-to="' + esc(to.name) + '" data-refs="' + e.refs + '"><title>' + esc(from.name + " → " + to.name + "：" + e.refs + " 处引用" + (e.violation ? "（越界）" : "") + "\n首个引用点：" + e.file) + "</title>");
      out.push('<rect x="' + (cx + 1) + '" y="' + (cy + 1) + '" width="' + (CW - 2) + '" height="' + (RH - 2) + '" rx="3" fill="rgba(37,99,235,' + a.toFixed(2) + ')"' + (e.violation ? ' stroke="#dc2626" stroke-width="1.5"' : "") + "/>");
      out.push('<text x="' + (cx + CW / 2) + '" y="' + (cy + RH / 2 + 3.5) + '" text-anchor="middle" class="mnum" fill="' + (a > 0.5 ? "#fff" : "#1e293b") + '">' + e.refs + "</text></g>");
    }
  }
  for (let i = 0; i < nr; i++) {
    const c = rows[i];
    const w = wide.indexOf(c.name) >= 0;
    out.push('<g class="node mhead' + (w ? " wide" : "") + '" data-view="caps" data-name="' + esc(c.name) + '" data-axis="row" transform="translate(0,' + yOf[i] + ')"><title>' + esc(c.name + " · " + ringFull(c.layer) + " · 出 " + c.out + " 边 / " + (outRefs.get(c.name) || 0) + " 处引用 · 入 " + (inRefs.get(c.name) || 0) + " 处") + "</title>");
    out.push('<rect width="' + (LAB - 6) + '" height="' + RH + '" fill="transparent"/>');
    out.push('<text x="' + (LAB - 8) + '" y="' + (RH / 2 + 4) + '" text-anchor="end" class="rname">' + esc(c.name) + "</text>");
    out.push('<text x="' + (LAB + nc * CW + 8) + '" y="' + (RH / 2 + 4) + '" class="tot">' + (outRefs.get(c.name) || 0) + "</text></g>");
  }
  for (let j = 0; j < nc; j++) {
    const c = cols[j];
    out.push('<g class="node mhead" data-view="caps" data-name="' + esc(c.name) + '" data-axis="col" transform="translate(' + (xOf(j) + CW / 2) + "," + (TOP - 6) + ')"><title>' + esc(c.name + " · 入 " + (inRefs.get(c.name) || 0) + " 处引用") + "</title>");
    out.push('<text transform="rotate(-90)" text-anchor="start" class="cname">' + esc(c.name) + "</text></g>");
  }
  out.push('<text x="' + (LAB + nc * CW + 8) + '" y="' + (TOP - 8) + '" class="band">出</text>');
  out.push("</svg>");
  const noOut = nodes.filter((c) => !(outRefs.get(c.name) > 0)).map((c) => c.name);
  const noIn = nodes.filter((c) => !(inRefs.get(c.name) > 0)).map((c) => c.name);
  const rank = edges.slice().sort((a, b) => b.refs - a.refs || a.from.localeCompare(b.from)).slice(0, 20);
  out.push('<div class="capnote"><span class="band">行 = 引用方，列 = 被引用方；格内是引用处数，颜色越深越多，红框是门禁会拦的边；右侧「出」是该单元对其它单元的总引用处数。</span>');
  out.push('<span class="band">环间边：环 3→2 ' + (sums["3>2"][0] || 0) + " 条 / " + (sums["3>2"][1] || 0) + " 处 · 环 3→1 " + (sums["3>1"][0] || 0) + " 条 / " + (sums["3>1"][1] || 0) + " 处 · 环 2→1 " + (sums["2>1"][0] || 0) + " 条 / " + (sums["2>1"][1] || 0) + " 处 · 同环 0 条</span>");
  out.push('<span class="band">只画有出边/入边的单元：无跨单元出边 ' + noOut.length + " 个（" + esc(noOut.join("、")) + "）；无入边 " + noIn.length + " 个（" + esc(noIn.join("、")) + "）。</span></div>");
  out.push('<ol class="rank"><li class="rh">引用处数前 ' + rank.length + " 的强边（点单元名看详情）</li>");
  for (const e of rank) {
    out.push("<li><a class=\"node\" data-view=\"caps\" data-name=\"" + esc(e.from) + "\">" + esc(e.from) + "</a> → <a class=\"node\" data-view=\"caps\" data-name=\"" + esc(e.to) + "\">" + esc(e.to) + "</a><span class=\"n\">" + e.refs + " 处 · " + esc(e.file) + "</span></li>");
  }
  out.push("</ol></div>");
  return out.join("");
}

/** 全景：entry 一列装配三层，主链 presentation → units → kernel；只画聚合数。 */
function overviewSvg(meta, entry) {
  const W = 1080;
  const ex = 20;
  const ew = 250;
  const bx = 300;
  const bw = 750;
  const groups = new Map();
  for (const n of entry.nodes) {
    if (!groups.has(n.layer)) groups.set(n.layer, []);
    groups.get(n.layer).push(n.name);
  }
  const segNames = { 0: "值层", 1: "判定层", 2: "装配层", 3: "编排层" };
  const entryLines = [...groups.keys()].sort((a, b) => b - a).map((k) => segNames[k] + "：" + groups.get(k).sort().join(" · "));
  const bands = [
    { view: "pres", y: 20, h: 100, title: "presentation · " + meta.screens + " 屏 / " + meta.screenFiles + " 个文件", lines: ["每屏只取清单里的单元：" + meta.unitCount + " 个单元、" + meta.cross.screenEdges + " 处屏→单元引用（docs/presentation/inbound.yaml）"], tag: "点这里看屏面入站面" },
    { view: "caps", y: 164, h: 110, title: "units · " + meta.caps + " 单元 / " + meta.files + " 个文件", lines: ["单元之间 " + meta.edges + " 条依赖边，环间只向内（layers.yaml）", "对外只经 kernel 的 " + meta.cross.faces + " 个 api 面（" + meta.cross.capKernel + " 处取用）"], tag: "点这里看单元环" },
    { view: "mech", y: 318, h: 110, title: "kernel · " + meta.mech + " 机制 / " + meta.mechLayers + " 层", lines: ["机制之间 " + meta.mechEdges + " 条边，严格下行、无环（docs/kernel/mechanisms.yaml）", "对外面按概念切成 " + meta.cross.faces + " 个，内核内部不绕道面"], tag: "点这里看机制分层" },
  ];
  const out = [];
  out.push('<svg viewBox="0 0 ' + W + " " + (bands[2].y + bands[2].h + 30) + '" width="' + W + '" height="' + (bands[2].y + bands[2].h + 30) + '" role="img" aria-label="跨层全景">');
  out.push('<defs><marker id="oar" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="8" markerHeight="8" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#475569"/></marker></defs>');
  const totalH = bands[2].y + bands[2].h - 20;
  out.push('<g class="node" data-view="entry" data-name="" transform="translate(' + ex + "," + 20 + ')"><title>entry：启动流程，装配下游三层</title>');
  out.push('<rect width="' + ew + '" height="' + totalH + '" rx="12" fill="#0f172a" stroke="#0f172a"/>');
  out.push('<text x="14" y="26" style="font-size:14px;fill:#fff;font-weight:600">entry · ' + meta.entry + " 个模块 / " + meta.entrySegments + " 段</text>");
  entryLines.forEach((line, i) => {
    out.push('<text x="14" y="' + (52 + i * 34) + '" style="font-size:11px;fill:#cbd5e1">' + esc(line) + "</text>");
  });
  out.push('<text x="14" y="' + (52 + entryLines.length * 34) + '" style="font-size:11px;fill:#94a3b8">编排层 main 只串流程</text>');
  out.push('<text x="14" y="' + (78 + entryLines.length * 34) + '" style="font-size:11px;fill:#cbd5e1">装配 ' + meta.cross.entryPres + " 屏 / " + meta.cross.entryCap + ' 个单元</text>');
  out.push('<text x="14" y="' + (78 + entryLines.length * 34 + 20) + '" style="font-size:11px;fill:#cbd5e1">' + meta.cross.entryKernel + ' 个模块取 kernel 对外面</text>');
  out.push('<text x="14" y="' + (totalH - 16) + '" style="font-size:11px;fill:#64748b">点这里看入口分段</text>');
  out.push("</g>");
  const arrow = (x1, y1, x2, y2, label, at) => {
    out.push('<path d="M' + x1 + "," + y1 + " C" + (x1 + (x2 - x1) * 0.45) + "," + y1 + " " + (x1 + (x2 - x1) * 0.55) + "," + y2 + " " + x2 + "," + y2 + '" fill="none" stroke="#475569" stroke-opacity="0.55" stroke-width="1.4" marker-end="url(#oar)"/>');
    if (label) out.push('<text x="' + at[0] + '" y="' + at[1] + '" style="font-size:11px;fill:#475569">' + esc(label) + "</text>");
  };
  bands.forEach((b, i) => {
    out.push('<g class="node" data-view="' + b.view + '" data-name="" transform="translate(' + bx + "," + b.y + ')"><title>' + esc(b.title) + "</title>");
    out.push('<rect width="' + bw + '" height="' + b.h + '" rx="12" fill="' + (i % 2 ? "#f8fafc" : "#ffffff") + '" stroke="#cbd5e1"/>');
    out.push('<text x="16" y="26" style="font-size:14px;fill:#0f172a;font-weight:600">' + esc(b.title) + "</text>");
    b.lines.forEach((line, k) => {
      out.push('<text x="16" y="' + (50 + k * 22) + '" style="font-size:11.5px;fill:#475569">' + esc(line) + "</text>");
    });
    out.push('<text x="' + (bw - 16) + '" y="' + (b.h - 14) + '" text-anchor="end" style="font-size:11px;fill:#94a3b8">' + esc(b.tag) + "</text>");
    out.push("</g>");
    arrow(ex + ew, b.y + b.h / 2, bx, b.y + b.h / 2);
  });
  arrow(bx + bw * 0.5, bands[0].y + bands[0].h, bx + bw * 0.5, bands[1].y, meta.cross.screenEdges + " 处屏→单元", [bx + bw * 0.5 + 8, bands[0].y + bands[0].h + 20]);
  arrow(bx + bw * 0.5, bands[1].y + bands[1].h, bx + bw * 0.5, bands[2].y, meta.cross.capKernel + " 处 → " + meta.cross.faces + " 个对外面", [bx + bw * 0.5 + 8, bands[1].y + bands[1].h + 22]);
  out.push("</svg>");
  return out.join("");
}

/** 整个页面：样式与点击逻辑都内联，双击即可看（产物不进库）。 */
function page(data) {
  const m = data.meta;
  const svg = {
    overview: overviewSvg(m, data.entry),
    caps: capsSvg(data.caps, data.edges, m.wide),
    mech: bandSvg({ nodes: data.mech.nodes, edges: data.mech.edges }, { nodeW: 165, nodeH: 44, wide: m.wideMech, bandLabel: (l) => "第 " + l + " 层", aria: "kernel 机制分层图", id: "mech", view: "mech" }),
    entry: bandSvg({ nodes: data.entry.nodes, edges: data.entry.edges }, { nodeW: 140, nodeH: 44, wide: m.wideEntry, bandLabel: (l) => "第 " + l + " 段", aria: "入口分段图", id: "entry", view: "entry" }),
    pres: presSvg(data.pres),
  };
  const stageHtml = VIEWS.map((v) => '<div class="view" data-view="' + v.key + '">' + svg[v.key] + "</div>").join("");
  const tabs = VIEWS.map((v) => '<button data-tab="' + v.key + '"' + (v.key === "overview" ? ' class="on"' : "") + ">" + v.tab + "</button>").join("");
  data.hints = HINTS;
  const json = JSON.stringify(data).replace(/<\/script/gi, "<\\/script").replace(/</g, "\\u003c");
  const style = [
    "*{box-sizing:border-box}body{margin:0;font:14px/1.5 -apple-system,'Segoe UI','Microsoft YaHei',sans-serif;color:#0f172a;background:#f1f5f9}",
    "header{position:sticky;top:0;z-index:20;padding:12px 20px;background:#0f172a;color:#e2e8f0;display:flex;gap:12px;align-items:center;flex-wrap:wrap}",
    "header h1{margin:0;font-size:16px;font-weight:600;letter-spacing:.02em}",
    "header .n{font-size:12px;color:#93a3b8}header .n b{color:#fff;font-weight:600}",
    "#tabs{display:flex;gap:6px;margin-left:auto}",
    "#tabs button{font:12px/1 inherit;padding:6px 12px;border-radius:999px;border:1px solid #334155;background:#1e293b;color:#cbd5e1;cursor:pointer}",
    "#tabs button:hover{border-color:#64748b}#tabs button.on{background:#e2e8f0;color:#0f172a;border-color:#e2e8f0;font-weight:600}",
    "#fit{font:12px/1 inherit;padding:6px 12px;border-radius:999px;border:1px solid #475569;background:transparent;color:#cbd5e1;cursor:pointer}",
    "#fit.on{background:#334155;border-color:#94a3b8;color:#fff}",
    "#stage.fit svg{width:100%;max-width:100%;height:auto}",
    "main{display:grid;grid-template-columns:minmax(0,1fr) 380px;gap:14px;padding:14px;align-items:start;max-width:1560px;margin:0 auto}",
    "main>section{min-width:0}",
    "#stage svg{display:block;max-width:100%;height:auto;margin:0 auto}",
    "#stage svg.wide{max-width:none}",
    "#stage{background:#fff;border:1px solid #e2e8f0;border-radius:10px;overflow:auto;padding:6px;min-width:0}",
    "#panel{background:#fff;border:1px solid #e2e8f0;border-radius:10px;padding:14px;position:sticky;top:14px;max-height:calc(100vh - 28px);overflow:auto}",
    ".view[hidden]{display:none}",
    ".band{font-size:10.5px;fill:#64748b;letter-spacing:.06em}",
    ".nm{font-size:12.5px;fill:#0f172a;font-weight:600}.sub{font-size:10px;fill:#64748b}",
    ".badge{font-size:9.5px;fill:#fff;font-weight:700}",
    "g.node{cursor:pointer}g.node:hover rect{fill-opacity:.85}",
    "path.edge{pointer-events:stroke}path.edge.hot{stroke-opacity:1;stroke:#2563eb}",
    "#crumbs{font-size:12px;color:#94a3b8;display:flex;gap:6px;align-items:center;min-width:0;max-width:100%;overflow:hidden;white-space:nowrap}",
    "#crumbs .sep{color:#64748b}",
    "#back{font:12px/1 inherit;padding:6px 10px;border-radius:8px;border:1px solid #475569;background:transparent;color:#cbd5e1;cursor:pointer;white-space:nowrap}",
    "#back:hover:not(:disabled){border-color:#94a3b8;color:#fff}",
    "#back:disabled{opacity:.3;cursor:default}",
    "#crumbs a{color:#60a5fa;cursor:pointer;text-decoration:none}#crumbs a:hover{text-decoration:underline}",
    "#panel h2{margin:0 0 6px;font-size:15px}#panel h3{margin:14px 0 6px;font-size:12px;color:#475569;letter-spacing:.04em}",
    "#panel .path{font:11.5px/1.45 ui-monospace,Consolas,monospace;color:#0f172a;background:#f8fafc;border:1px solid #e2e8f0;border-radius:6px;padding:6px;word-break:break-all}",
    "#panel dl{margin:0;font-size:12.5px}#panel dt{font-weight:600;color:#334155;margin-top:8px}#panel dd{margin:2px 0 0;color:#334155}",
    "#panel ul{margin:0;padding-left:16px;font-size:12px;color:#b91c1c}#panel ul li{margin:3px 0}",
    ".tag{display:inline-block;font-size:10.5px;padding:1px 7px;border-radius:999px;background:#e2e8f0;color:#334155;margin-right:6px}",
    ".tag.red{background:#fee2e2;color:#b91c1c}.tag.amber{background:#fef3c7;color:#92400e}",
    ".hint{font-size:12px;color:#64748b}",
    ".legend{font-size:11.5px;color:#475569;display:flex;gap:12px;flex-wrap:wrap;margin:4px 0 0}",
    ".legend i{display:inline-block;width:10px;height:10px;border-radius:3px;margin-right:4px;vertical-align:-1px}",
    "svg .fnode{cursor:pointer}svg .fnode rect{fill:#fff;stroke:#cbd5e1}svg .fnode:hover rect{fill:#eff6ff}",
    "svg .fnode.hit rect{stroke:#dc2626;stroke-width:2}",
    "svg .fline{stroke:#94a3b8;stroke-opacity:.45;fill:none}",
    ".caps svg.matrix{display:block;margin:0 auto}",
    ".capnote{display:flex;flex-wrap:wrap;gap:2px 18px;padding:8px 12px;border-top:1px solid #e2e8f0}",
    ".caps .rank{columns:2;column-gap:26px;margin:8px 12px 14px;padding:0 0 0 18px;font-size:12px;color:#334155}",
    ".caps .rank li{margin:3px 0;break-inside:avoid}",
    ".caps .rank li.rh{list-style:none;margin-left:-18px;font-weight:600;color:#0f172a;margin-bottom:4px}",
    ".caps .rank .n{color:#94a3b8;font-size:10.5px;margin-left:6px}",
    ".caps .rank a{color:#1d4ed8;cursor:pointer;text-decoration:none}.caps .rank a:hover{text-decoration:underline}",
    "svg .rname{font-size:11.5px;fill:#0f172a}svg .cname{font-size:10px;fill:#334155}",
    "svg .rband{font-size:10.5px;fill:#94a3b8;letter-spacing:.02em}",
    "svg .tot{font-size:10px;fill:#94a3b8}svg g.node.wide .rname{fill:#b45309;font-weight:700}",
    "svg .mnum{font-size:9px;font-weight:600;pointer-events:none}svg .mcell{cursor:pointer}",
    "svg .mcell.dim{opacity:.2}svg .mcell.hot rect{stroke:#2563eb;stroke-width:2}",
  ].join("");
  const java = [
    "var D=JSON.parse(document.getElementById('graph-data').textContent),HINTS=D.hints;",
    "var TAB={overview:'全景',caps:'单元环',mech:'kernel 机制',entry:'入口',pres:'屏面'};",
    "var stage=document.getElementById('stage'),panel=document.getElementById('panel'),crumbs=document.getElementById('crumbs');",
    "var home=stage.innerHTML,cur={t:'home',v:'overview'},hist=[];",
    "var backBtn=document.getElementById('back');",
    "var stageH=document.getElementById('stage');",
    "var FA={};",
    "D.caps.forEach(function(c){c.files.forEach(function(f){FA[f.path]={view:'caps',node:c.name};});});",
    "D.mech.nodes.forEach(function(c){c.files.forEach(function(f){FA[f.path]={view:'mech',node:c.name};});});",
    "D.entry.nodes.forEach(function(c){c.files.forEach(function(f){FA[f.path]={view:'entry',node:c.name};});});",
    "D.pres.screens.forEach(function(s){s.files.forEach(function(f){FA[f.path]={view:'pres',node:s.file};});});",
    "function esc(s){return String(s==null?'':s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/\"/g,'&quot;');}",
    "function cut(s,n){s=String(s==null?'':s);return s.length>n?s.slice(0,n-1)+'…':s;}",
    "function mhot(f,t){stage.querySelectorAll('.mcell').forEach(function(c){var on=(f&&c.getAttribute('data-from')===f)||(t&&c.getAttribute('data-to')===t);c.classList.toggle('hot',!!on);c.classList.toggle('dim',!on);});}",
    "function mclear(){stage.querySelectorAll('.mcell').forEach(function(c){c.classList.remove('hot');c.classList.remove('dim');});}",
    "function issues(arr){if(!arr||!arr.length)return '<p class=tag>无命中</p>';return '<ul>'+arr.map(function(t){return '<li>'+esc(t)+'</li>';}).join('')+'</ul>';}",
    "function crumb(parts){crumbs.innerHTML=parts.map(function(p,i){return i? '<span>›</span> <a data-go=\"'+esc(p[1])+'\">'+esc(p[0])+'</a>':'<a data-go=\"'+esc(p[1])+'\">'+esc(p[0])+'</a>';}).join(' ');}",
    "function show(v){",
    " stage.querySelectorAll('.view').forEach(function(d){d.hidden=d.getAttribute('data-view')!==v;});",
    " [].forEach.call(document.querySelectorAll('#tabs button'),function(b){b.className=b.getAttribute('data-tab')===v?'on':'';});",
    " stage.scrollTop=0;stage.scrollLeft=0;}",
    "function drawHome(){stage.innerHTML=home;show(cur.v);panel.innerHTML=HINTS[cur.v];crumbs.innerHTML='';wire();}",
    "function draw(){backBtn.disabled=!hist.length;if(cur.t==='node')drawNode();else if(cur.t==='file')drawFile();else drawHome();}",
    "function nav(st){if(st.t===cur.t&&(st.t==='file'?st.p===cur.p:st.v===cur.v&&st.n===cur.n))return;hist.push(cur);if(hist.length>80)hist.shift();cur=st;draw();}",
    "function back(){if(!hist.length)return;cur=hist.pop();draw();}",
    "function viewKey(v){return v==='caps'?D.caps:v==='mech'?D.mech.nodes:v==='entry'?D.entry.nodes:D.pres.screens;}",
    "function nodeOf(view,name){return viewKey(view).filter(function(x){return (x.file||x.name)===name;})[0];}",
    "function fileGraph(files,edges,prefix){var n=files.length,cols=Math.min(4,Math.max(1,Math.ceil(Math.sqrt(n)))),rows=Math.ceil(n/cols);",
    " var CW=300,CH=78,GX=34,GY=26,W=cols*CW+(cols-1)*GX+40,H=rows*CH+(rows-1)*GY+40;",
    " var P=new Map();files.forEach(function(f,i){P.set(f.path,{x:20+(i%cols)*(CW+GX),y:20+Math.floor(i/cols)*(CH+GY)});});",
    " var out=['<svg viewBox=\"0 0 '+W+' '+H+'\" width=\"'+W+'\" height=\"'+H+'\"><defs><marker id=\"fa\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto\"><path d=\"M0,0 L8,4 L0,8 z\" fill=\"#94a3b8\"/></marker></defs>'];",
    " (edges||[]).forEach(function(e){var a=P.get(e.from),b=P.get(e.to);if(!a||!b)return;",
    "  var x1=a.x+CW/2,y1=a.y+56,x2=b.x+CW/2,y2=b.y;var dy=y2-y1;",
    "  out.push('<path class=fline d=\"M'+x1+','+y1+' C'+x1+','+(y1+dy*0.5)+' '+x2+','+(y2-dy*0.5)+' '+x2+','+y2+'\" stroke-width=\"'+(0.8+Math.min(e.refs,8)*0.18)+'\" marker-end=\"url(#fa)\" data-from=\"'+esc(e.from)+'\" data-to=\"'+esc(e.to)+'\"/>');});",
    " files.forEach(function(f){var p=P.get(f.path);var short=f.path;if(prefix&&f.path.indexOf(prefix)===0){short=f.path.slice(prefix.length).replace(/^\\//,'');if(short==='.rs')short=prefix.split('/').pop()+'.rs';}",
    "  out.push('<g class=\"fnode'+(f.issues&&f.issues.length?' hit':'')+'\" data-file=\"'+esc(f.path)+'\" transform=\"translate('+p.x+','+p.y+')\"><rect width=\"'+CW+'\" height=\"56\" rx=\"8\"/><text x=\"10\" y=\"18\" style=\"font-size:12px;fill:#0f172a\">'+esc(cut(short,30))+'</text><text x=\"10\" y=\"34\" style=\"font-size:10px;fill:#64748b\">'+(f.role==='test'?'测试':f.role==='support'?'测试支撑':'实现')+' · '+f.total+' 行'+(f.soft?' · 超软线':'')+'</text><text x=\"10\" y=\"48\" style=\"font-size:9.5px;fill:#94a3b8\">'+esc(cut(f.head&&f.head['目的']||'',26))+'</text></g>');});",
    " out.push('</svg>');return out.join('');}",
    "function drawNode(){var view=cur.v,name=cur.n;stage.scrollTop=0;stage.scrollLeft=0;",
    " var n=nodeOf(view,name);if(!n)return;",
    " var prefix=view==='caps'?n.dir:view==='mech'?'src/kernel/'+name:view==='entry'?'src/entry/':view==='pres'?'src/presentation/':'';",
    " var fe=(view==='caps'?D.fe[name]:view==='mech'?D.mech.fe[name]:view==='entry'?D.entry.fe[name]:null)||[];",
    " crumb([['全景',''],[TAB[view],'view:'+view],[view==='pres'?n.name:n.name,view==='pres'?'v:'+view+':'+n.file:'v:'+view+':'+name]]);",
    " stage.innerHTML=fileGraph(n.files,fe,prefix);",
    " var tags=view==='entry'?'<span class=tag>第 '+n.layer+' 段</span>':view==='pres'?'<span class=tag>取 '+n.units.length+' 个单元</span>':'<span class=tag>环 '+n.layer+'</span>';",
    " panel.innerHTML='<h2>'+esc(view==='pres'?n.name:n.name)+'</h2>'+tags+'<span class=tag>'+n.files.length+' 个文件</span>'",
    "  +(view==='pres'?'<div class=path style=\"margin-top:8px\">'+esc(n.file)+'</div><h3>入站面单元</h3><p class=hint>'+esc(n.units.join(' · '))+'</p>':'')",
    "  +'<h3>命中问题</h3>'+issues(n.issues)+'<h3>提示</h3><p class=hint>点文件看四槽；悬停文件高亮它的依赖。</p>';",
    " bindFile();}",
    "function drawFile(){var path=cur.p;stage.scrollTop=0;stage.scrollLeft=0;var at=FA[path];if(!at)return;var n=nodeOf(at.view,at.node);if(!n)return;",
    " var f=n.files.filter(function(x){return x.path===path;})[0];if(!f)return;var h=f.head||{};",
    " crumb([['全景',''],[TAB[at.view],'view:'+at.view],[at.view==='pres'?n.name:n.name,'v:'+at.view+':'+at.node],[f.path.split('/').pop(),'file:'+path]]);",
    " var dl=['目的','管','不管','联动'].map(function(k){return '<dt>'+k+'</dt><dd>'+esc(h[k]||'（缺）')+'</dd>';}).join('');",
    " panel.innerHTML='<h2>'+esc(f.path.split('/').pop())+'</h2><div class=path>'+esc(f.path)+'</div>'+",
    "  '<div style=\"margin-top:8px\"><span class=tag>'+(f.role==='test'?'测试':f.role==='support'?'测试支撑':'实现')+'</span><span class=tag>'+f.total+' 行</span>'+(f.soft?'<span class=\"tag amber\">超软线</span>':'')+(f.issues&&f.issues.length?'<span class=\"tag red\">'+f.issues.length+' 处命中</span>':'')+'</div>'+",
    "  '<h3>文件头四槽</h3><dl>'+dl+'</dl><h3>命中问题</h3>'+issues(f.issues);}",
    "function bindFile(){var nodes=stage.querySelectorAll('.fnode'),lines=stage.querySelectorAll('.fline');",
    " nodes.forEach(function(g){g.addEventListener('click',function(){nav({t:'file',p:g.getAttribute('data-file')});});",
    "  g.addEventListener('mouseenter',function(){var p=g.getAttribute('data-file');lines.forEach(function(l){if(l.getAttribute('data-from')===p||l.getAttribute('data-to')===p)l.setAttribute('stroke','#2563eb');});});",
    "  g.addEventListener('mouseleave',function(){lines.forEach(function(l){l.setAttribute('stroke','');});});});}",
    "function wire(){",
    " stage.querySelectorAll('g.node').forEach(function(g){var view=g.getAttribute('data-view'),name=g.getAttribute('data-name');",
    "  g.addEventListener('click',function(){nav(name?{t:'node',v:view,n:name}:{t:'home',v:view});});",
    "  g.addEventListener('mouseenter',function(){if(!name)return;stage.querySelectorAll('path.edge').forEach(function(p){if(p.getAttribute('data-from')===name||p.getAttribute('data-to')===name)p.classList.add('hot');});});",
    "  g.addEventListener('mouseleave',function(){stage.querySelectorAll('path.edge').forEach(function(p){p.classList.remove('hot');});});});",
    " document.querySelectorAll('#tabs button').forEach(function(b){b.addEventListener('click',function(){nav({t:'home',v:b.getAttribute('data-tab')});});});",
    " stage.querySelectorAll('.mcell').forEach(function(c){c.addEventListener('mouseenter',function(){mhot(c.getAttribute('data-from'),c.getAttribute('data-to'));});c.addEventListener('mouseleave',mclear);c.addEventListener('click',function(){nav({t:'node',v:'caps',n:c.getAttribute('data-from')});});});",
    " stage.querySelectorAll('a.node').forEach(function(a){a.addEventListener('click',function(){nav({t:'node',v:a.getAttribute('data-view'),n:a.getAttribute('data-name')});});});",
    " stage.querySelectorAll('.mhead').forEach(function(g){var nm=g.getAttribute('data-name'),ax=g.getAttribute('data-axis');g.addEventListener('mouseenter',function(){mhot(ax==='row'?nm:null,ax==='col'?nm:null);});g.addEventListener('mouseleave',mclear);});",
    "}",
    "document.addEventListener('click',function(ev){var a=ev.target.closest('a[data-go]');if(!a)return;var v=a.getAttribute('data-go');",
    " if(!v){nav({t:'home',v:'overview'});}else if(v.indexOf('view:')===0){nav({t:'home',v:v.slice(5)});}else if(v.indexOf('v:')===0){var r=v.slice(2).split(':');nav({t:'node',v:r[0],n:v.slice(2+r[0].length+1)});}else if(v.indexOf('file:')===0){nav({t:'file',p:v.slice(5)});}});",
    "backBtn.addEventListener('click',function(){back();});",
    "document.addEventListener('keydown',function(ev){if(ev.key==='Escape'&&hist.length){back();}});",
    "document.getElementById('fit').addEventListener('click',function(){this.classList.toggle('on');stage.classList.toggle('fit');});",
    "draw();",
  ].join("\n");
  return [
    "<!doctype html>",
    '<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">',
    "<title>Solomni 结构图</title><style>" + style + "</style></head><body>",
    "<header><h1>Solomni 结构图</h1>",
    '<span class="n">单元 <b>' + m.caps + "</b> · 机制 <b>" + m.mech + "</b> · 入口 <b>" + m.entry + "</b> · 屏 <b>" + m.screens + "</b> · 依赖边 <b>" + m.edges + " / " + m.mechEdges + " / " + m.entryEdges + "</b> · 命中 <b>" + m.issues + "</b></span>",
    '<button id="back" type="button" disabled>← 返回</button><div id="crumbs"></div>',
    '<nav id="tabs">' + tabs + '</nav><button id="fit" type="button">适应宽度</button></header>',
    "<main><section><div id=stage>" + stageHtml + "</div>",
    '<div class="legend"><span><i style="background:#fff;border:1px solid #cbd5e1"></i>常规</span><span><i style="background:#fff;border:1px dashed #f59e0b"></i>扇出最大的三个</span><span><i style="background:#fff;border:1px solid #dc2626"></i>门禁命中</span><span><i style="background:#dc2626;height:2px"></i>越界依赖</span><span>点节点下钻 · 点文件看四槽 · 左上「返回」或 Esc 逐级回退 · 宽图可横向滚动，右上「适应宽度」缩到一屏</span><span>由门禁 <b>structure/graph</b> 每次覆盖；层级以四张登记表为准</span></div></section>',
    "<aside id=panel>" + HINTS.overview + "</aside></main>",
    '<script id="graph-data" type="application/json">' + json + "</script>",
    "<script>" + java + "</script>",
    "</body></html>",
  ].join("\n");
}

/** 读上一版产物里的计数，只用于「与上次比」的差异摘要（产物本身不进库）。 */
function previousMeta(file) {
  if (!fs.existsSync(file)) return null;
  const m = /"meta":\{[^}]*\}/.exec(fs.readFileSync(file, "utf8"));
  if (!m) return null;
  try {
    return JSON.parse(m[0].slice(7));
  } catch (e) {
    return null;
  }
}

/** 计数摘要：变化量只在有上一版时给，避免把「首次生成」说成增长。 */
function summary(meta, before) {
  const base = "结构图：Structure.html " + (before ? "已刷新" : "已生成") + "（" + meta.caps + " 单元 /" + meta.mech + " 机制 / " + meta.entry + " 入口模块 / " + meta.screens + " 屏；依赖边 " + meta.edges + " / " + meta.mechEdges + " / " + meta.entryEdges + "；命中 " + meta.issues + "）";
  if (!before) return base;
  const parts = [];
  for (const [key, label] of [["caps", "单元"], ["mech", "机制"], ["entry", "入口模块"], ["screens", "屏"], ["edges", "单元边"], ["mechEdges", "机制边"], ["entryEdges", "入口边"], ["issues", "命中"]]) {
    const d = meta[key] - (before[key] || 0);
    if (d) parts.push(label + (d > 0 ? "+" : "") + d);
  }
  return base + (parts.length ? "；比上次 " + parts.join("、") : "；与上次一致");
}

/** 生成并覆盖产物：内容不变就不写，返回值给门禁当回执。 */
function write(root) {
  const data = collect(root);
  const file = path.join(root, OUT);
  const html = page(data) + "\n";
  const before = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
  const prev = previousMeta(file);
  if (before !== html) fs.writeFileSync(file, html, "utf8");
  return { changed: before !== html, meta: data.meta, note: summary(data.meta, prev) };
}

/** 门禁步：永远 pass，只写产物与回执——结构图是「让判据可见」，不是判据本身。 */
function graph(root) {
  const r = write(root);
  return { problems: [], notes: [r.note] };
}

module.exports = { collect, page, render: (root) => page(collect(root)), write, graph };

/** 命令行：单独重画一次，便于本地看效果。 */
if (require.main === module) {
  const root = path.resolve(__dirname, "..", "..", "..");
  const r = write(root);
  console.log(r.note);
}
