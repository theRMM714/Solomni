/**
 * 结构审查·单元环门禁（H1 环方向、H2 纯层）：环是架构地位的唯一权威，写在 layers.yaml。
 * 四环：0 kernel（地基）、1 纯（无 IO / 无状态 / 无端口 / 可穷举测试）、2 状态与机制、3 用例；
 * 环 4 是适配器（presentation/、entry/），不入本表，由 entry-tiers.js 与 dependency.js 管。
 * H1 硬判：跨单元边只能指向严格更低的环；同环只允许指向 rank 更低的单元——rank 由同环边算，
 *          只回执、不设预算（同环先后是机制栈的真实顺序，不是需要消灭的耦合）。
 * H2 硬判：环 1 的单元不得出现 IO（std::fs/net/process/env/thread、tokio、reqwest、sqlx）、
 *          ports/ 目录、可变全局（static mut / Mutex / RwLock / OnceLock / lazy_static），也不得依赖 kernel 的机制面
 *          （只放行 crate::kernel::contracts 与 kernel::api::{types,note} 的纯契约值）。
 * 只回执：同环 rank 序、扇出、纯转发边。H3（跨切片共用值应下沉环 1）与 N2 在 P3 接入。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { stripTestModules } = require("./dependency.js");
const { BUCKETS, unitDir } = require("./unitpath.js");

const TABLE = "layers.yaml";
const RINGS = new Map([[0, "地基"], [1, "纯"], [2, "状态与机制"], [3, "用例"]]);

/** 盘上的单元名：三个角色桶（pure / resource / business）里各再收一层的单元。 */
const dirsOf = (root) => {
  const base = path.join(root, "src", "units");
  const out = [];
  for (const e of fs.readdirSync(base, { withFileTypes: true })) {
    if (!e.isDirectory()) continue;
    if (!BUCKETS.has(e.name)) { out.push(e.name); continue; }
    for (const p of fs.readdirSync(path.join(base, e.name), { withFileTypes: true })) if (p.isDirectory()) out.push(p.name);
  }
  return out.sort();
};

const rsFilesOf = (dir) => {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFilesOf(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

/** 解析环表：一行一条「单元: 环号」，井号之后是注释。返回 Map 与表自身的格式问题。 */
function parseLayers(root) {
  const tiers = new Map();
  const problems = [];
  const file = path.join(root, TABLE);
  if (!fs.existsSync(file)) return { tiers, problems: ["环表：缺少 " + TABLE + "（单元分环的唯一权威）"] };
  let lineNo = 0;
  for (const raw of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    lineNo++;
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    const m = line.match(/^([a-z_][a-z0-9_]*):\s*(\d+)$/);
    if (!m) { problems.push("环表：" + TABLE + " 第 " + lineNo + " 行不是「单元: 环号」：" + raw.trim()); continue; }
    if (tiers.has(m[1])) { problems.push("环表：" + TABLE + " 里 " + m[1] + " 归了两次环"); continue; }
    tiers.set(m[1], Number(m[2]));
  }
  return { tiers, problems };
}

/** 环表形状：kernel 固定 0；每个环 1–3 非空；盘上与表双向全覆盖；环号只许 0–3。 */
function shapeProblems(root, tiers, dirs) {
  const problems = [];
  if (!tiers.has("kernel")) problems.push("环表：" + TABLE + " 必须给 kernel 归环 0");
  else if (tiers.get("kernel") !== 0) problems.push("环表：kernel 必须是环 0，实际 " + tiers.get("kernel"));
  for (const [name, ring] of tiers) {
    if (!RINGS.has(ring)) problems.push("环表：" + name + " 的环号 " + ring + " 越界——环 4 是适配器（presentation/、entry/），不入本表");
  }
  for (const d of dirs) if (!tiers.has(d)) problems.push("环表：src/units/" + d + "/ 没有在 " + TABLE + " 里归环");
  for (const [name, ring] of tiers) if (name !== "kernel" && !dirs.includes(name)) problems.push("环表：" + TABLE + " 里的 " + name + " 在 src/units/ 下没有目录");
  for (const [ring, label] of RINGS) {
    if (ring > 0 && ![...tiers.values()].includes(ring)) problems.push("环表：环 " + ring + "（" + label + "）没有任何单元——环是规则，空环说明这张表已经不描述架构");
  }
  const pure = path.join(root, "src", "units", "pure");
  if (fs.existsSync(pure)) {
    const real = fs.readdirSync(pure, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name).sort();
    const want = [...tiers].filter(([, r]) => r === 1).map(([n]) => n).sort();
    if (real.join(",") !== want.join(",")) {
      problems.push("环表：环 1 必须正好是 src/units/pure/ 的子目录；现在盘上 [" + real.join("、") + "]，表里 [" + want.join("、") + "]");
    }
  }
  return problems;
}

/** 依赖边：单元 A 的文件引用了 crate::units::[桶::]B（剔除注释与测试模块）。
 *  同一对边只记首个命中文件与引用处数；forward 记「这条边全是 pub use 再导出」。 */
const EDGE = new RegExp("crate::units::(?:" + [...BUCKETS].join("|") + "::)?([a-z_][a-z0-9_]*)", "g");
function edgesOf(root, dirs) {
  const edges = new Map();
  for (const cap of dirs) {
    for (const f of rsFilesOf(unitDir(root, cap))) {
      const text = stripTestModules(fs.readFileSync(f, "utf8"));
      for (const raw of text.split(/\r?\n/)) {
        if (!raw.trim() || raw.trim().startsWith("//")) continue;
        const forward = /^\s*pub\s+use\b/.test(raw);
        for (const m of raw.matchAll(EDGE)) {
          const to = m[1];
          if (to === cap || BUCKETS.has(to)) continue;
          const key = cap + ">" + to;
          const hit = edges.get(key);
          if (hit) {
            hit.refs++;
            if (!forward) hit.forward = false;
          } else edges.set(key, { file: f, refs: 1, forward: forward });
        }
      }
    }
  }
  return edges;
}

/** 同环 rank：只算同环边，rank(u) = 1 + max(rank(依赖))；返回 rank 序与环内成环的单元。 */
function ranksOf(tiers, edges) {
  const out = new Map();
  for (const key of edges.keys()) {
    const [from, to] = key.split(">");
    if (!tiers.has(from) || !tiers.has(to) || tiers.get(from) !== tiers.get(to)) continue;
    if (!out.has(from)) out.set(from, []);
    out.get(from).push(to);
  }
  const rank = new Map();
  const open = new Set();
  const cycles = new Set();
  const of = (u) => {
    if (rank.has(u)) return rank.get(u);
    if (open.has(u)) { cycles.add(u); return 0; }
    open.add(u);
    let top = 0;
    for (const v of out.get(u) || []) top = Math.max(top, of(v));
    open.delete(u);
    rank.set(u, top + 1);
    return top + 1;
  };
  for (const u of tiers.keys()) if (u !== "kernel") of(u);
  return { rank, cycles: [...cycles].sort() };
}

/** 单条边的 H1 判定：返回空串表示合法，否则返回问题措辞。 */
function edgeProblem(tiers, rank, from, to) {
  const a = tiers.get(from);
  const b = tiers.get(to);
  if (!RINGS.has(a) || !RINGS.has(b)) return "";
  if (b < a) return "";
  if (b > a) return "向上依赖（" + from + " 在环 " + a + "，依赖了环 " + b + " 的 " + to + "）";
  const ra = rank.get(from) || 1;
  const rb = rank.get(to) || 1;
  if (rb < ra) return "";
  return "同环逆序（" + from + " rank " + ra + " 依赖了 " + to + " rank " + rb + "，同环只能指向 rank 更低的单元）";
}

/** H2：纯层不得有 IO、端口、可变全局与 kernel 机制依赖；kernel 的纯契约值放行。 */
function pureProblems(root, unit) {
  const problems = [];
  const dir = unitDir(root, unit);
  if (fs.existsSync(path.join(dir, "ports"))) problems.push("纯层：环 1 的 " + unit + " 不得有 ports/ 目录（端口是 IO 的门，纯层不设门）");
  const bad = [
    [/^\s*use\s+(std::(?:fs|net|process|env|thread)|tokio|reqwest|sqlx)\b/, "IO 依赖"],
    [/^\s*(?:pub\s+)?static\s+mut\b/, "可变全局"],
    [/\blazy_static!?\s*\{/, "可变全局（lazy_static）"],
    [/\b(?:Mutex|RwLock|OnceLock|OnceCell|Atomic[A-Z]\w*)\b/, "状态/同步原语"],
  ];
  for (const f of rsFilesOf(dir)) {
    const text = stripTestModules(fs.readFileSync(f, "utf8"));
    const rel = f.slice(root.length + 1).replace(/\\/g, "/");
    text.split(/\r?\n/).forEach((line, i) => {
      if (!line.trim() || line.trim().startsWith("//")) return;
      for (const [re, why] of bad) if (re.test(line)) problems.push("纯层：环 1 的 " + unit + " 出现" + why + "，见 " + rel + ":" + (i + 1));
      // kernel 是环 0 的共享内核：纯契约值（contracts、api 的两个值面）可用；机制面与访问器不行。
      if (/^\s*use\s+crate::kernel\b/.test(line) && !/^\s*use\s+crate::kernel::(?:contracts\b|api::(?:types|note)\b)/.test(line))
        problems.push("纯层：环 1 的 " + unit + " 依赖 kernel 的机制面（只准 contracts 与 api::types/note 的纯值），见 " + rel + ":" + (i + 1));
    });
  }
  return problems;
}

/** 环判据：返回 problems（失败）与 notes（只报不拦）。 */
function layers(root) {
  const { tiers, problems: tableProblems } = parseLayers(root);
  const dirs = dirsOf(root);
  const problems = tableProblems.concat(shapeProblems(root, tiers, dirs));
  if (problems.length) return { problems, notes: [] };

  const edges = edgesOf(root, dirs);
  const { rank, cycles } = ranksOf(tiers, edges);
  for (const u of cycles) problems.push("环表：同环成环，牵涉 " + u + "（同环边必须是有向无环的先后序）");

  const fanout = new Map();
  for (const [key, info] of [...edges].sort()) {
    const [from, to] = key.split(">");
    fanout.set(from, (fanout.get(from) || 0) + 1);
    const why = edgeProblem(tiers, rank, from, to);
    if (why) problems.push("环表：单元 " + from + "（环 " + tiers.get(from) + "）" + why + "，见 " + info.file.slice(root.length + 1).replace(/\\/g, "/"));
  }
  for (const unit of [...tiers].filter(([, r]) => r === 1).map(([n]) => n).sort()) problems.push(...pureProblems(root, unit));

  const notes = [];
  for (const [ring, label] of RINGS) {
    if (ring === 0) continue;
    const members = [...rank].filter(([u]) => tiers.get(u) === ring).sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
    if (members.length) notes.push("环 " + ring + "（" + label + "）rank 序（只回执）：" + members.map(([u, r]) => u + " " + r).join("、"));
  }
  const forward = [...edges].filter(([, info]) => info.forward).map(([key]) => key.replace(">", " → ")).sort();
  if (forward.length) notes.push("环表：纯转发边（只 pub use 再导出，不当真依赖，只回执）：" + forward.join("、"));
  const rows = [...fanout].sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
  if (rows.length) notes.push("环表：扇出（只回执，不设预算）：" + rows.map(([c, n]) => c + " " + n).join("、"));
  return { problems, notes };
}

module.exports = { layers, parseLayers, edgesOf, dirsOf, rsFilesOf, ranksOf, edgeProblem, unitDir, RINGS };

/** 命令行：打印环表、同环 rank 与判据结果；环由人决定，没有 --write。 */
if (require.main === module) {
  const root = path.resolve(__dirname, "..", "..", "..");
  const { tiers } = parseLayers(root);
  const dirs = dirsOf(root);
  const edges = edgesOf(root, dirs);
  const { rank } = ranksOf(tiers, edges);
  const rows = [...tiers].filter(([k]) => k !== "kernel").sort((x, y) => x[1] - y[1] || x[0].localeCompare(y[0]));
  console.log("环  单元          依赖数  rank");
  for (const [cap, tier] of rows) {
    const n = [...edges.keys()].filter((k) => k.startsWith(cap + ">")).length;
    console.log(String(tier).padEnd(5) + cap.padEnd(14) + String(n).padEnd(8) + (rank.get(cap) || 0));
  }
  const r = layers(root);
  for (const p of r.problems) console.log("失败：" + p);
  for (const n of r.notes) console.log("回执：" + n);
  process.exit(r.problems.length ? 1 : 0);
}
