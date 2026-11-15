/**
 * 结构审查·能力分层门禁：能力只能依赖严格更低层，同层不得互相依赖。
 * 分层表在 layers.yaml（唯一权威）：kernel 是 0，能力从 1 起，层号连续。
 * 判据：① src/capabilities/<名字>/ 必须归层，表里的名字也必须有目录——归层不许留静默空洞；
 *       ② 依赖边的目标层必须严格小于来源层（向上与同层都失败）——这是「业务保持向下依赖」的机器判定；
 *       ③ 层号从 1 起连续、kernel 为 0，不许用 T37 这类号把单元藏到没人注意的位置。
 * 扇出只进 notes 不判红：预算 ≤5 是迁移目标，等 work / turn / collab 拆完再转硬。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { stripTestModules } = require("./dependency.js");

const TABLE = "layers.yaml";
const FANOUT_TARGET = 5;

const dirsOf = (root) => fs
  .readdirSync(path.join(root, "src", "capabilities"), { withFileTypes: true })
  .filter((e) => e.isDirectory())
  .map((e) => e.name)
  .sort();

const rsFilesOf = (dir) => {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFilesOf(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

/** 解析分层表：一行一条「名字: 层号」，`#` 之后是注释。返回 Map 与表自身的格式问题。 */
function parseLayers(root) {
  const tiers = new Map();
  const problems = [];
  const file = path.join(root, TABLE);
  if (!fs.existsSync(file)) return { tiers, problems: ["分层：缺少 " + TABLE + "（能力分层的唯一权威）"] };
  let lineNo = 0;
  for (const raw of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    lineNo++;
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    const m = line.match(/^([a-z_][a-z0-9_]*):\s*(\d+)$/);
    if (!m) { problems.push("分层：" + TABLE + " 第 " + lineNo + " 行不是「名字: 层号」：" + raw.trim()); continue; }
    if (tiers.has(m[1])) { problems.push("分层：" + TABLE + " 里 " + m[1] + " 归了两次层"); continue; }
    tiers.set(m[1], Number(m[2]));
  }
  return { tiers, problems };
}

/** 层号形状：kernel 必须 0；能力层从 1 起连续。 */
function shapeProblems(tiers, dirs) {
  const problems = [];
  if (!tiers.has("kernel")) problems.push("分层：" + TABLE + " 必须给 kernel 归层 0");
  else if (tiers.get("kernel") !== 0) problems.push("分层：kernel 必须是第 0 层，实际 " + tiers.get("kernel"));
  const caps = [...tiers.keys()].filter((k) => k !== "kernel");
  for (const d of dirs) if (!tiers.has(d)) problems.push("分层：src/capabilities/" + d + "/ 没有在 " + TABLE + " 里归层");
  for (const c of caps) if (!dirs.includes(c)) problems.push("分层：" + TABLE + " 里的 " + c + " 在 src/capabilities/ 下没有目录");
  const nums = caps.map((c) => tiers.get(c)).filter((n) => n > 0).sort((a, b) => a - b);
  const max = nums.length ? nums[nums.length - 1] : 0;
  for (let n = 1; n <= max; n++) if (!nums.includes(n)) problems.push("分层：能力层号必须从 1 起连续，缺第 " + n + " 层");
  for (const c of caps) if (tiers.get(c) < 1) problems.push("分层：能力 " + c + " 的层号必须 ≥ 1，实际 " + tiers.get(c));
  return problems;
}

/** 依赖边：能力 A 的文件引用了 crate::capabilities::B（剔除注释与测试模块），同一对只记一次。 */
function edgesOf(root, dirs) {
  const edges = new Map();
  for (const cap of dirs) {
    for (const f of rsFilesOf(path.join(root, "src", "capabilities", cap))) {
      const text = stripTestModules(fs.readFileSync(f, "utf8"))
        .split(/\r?\n/)
        .filter((l) => !l.trim().startsWith("//"))
        .join("\n");
      for (const m of text.matchAll(/crate::capabilities::([a-z_][a-z0-9_]*)/g)) {
        const to = m[1];
        if (to === cap) continue;
        if (!edges.has(cap + ">" + to)) edges.set(cap + ">" + to, f);
      }
    }
  }
  return edges;
}

/** 分层判据：返回 problems（失败）与 notes（只报不拦）。 */
function layers(root) {
  const { tiers, problems: tableProblems } = parseLayers(root);
  const dirs = dirsOf(root);
  const problems = tableProblems.concat(shapeProblems(tiers, dirs));
  if (problems.length) return { problems, notes: [] };

  const edges = edgesOf(root, dirs);
  const fanout = new Map();
  for (const [key, file] of [...edges].sort()) {
    const [from, to] = key.split(">");
    fanout.set(from, (fanout.get(from) || 0) + 1);
    const a = tiers.get(from);
    const b = tiers.get(to);
    if (b >= a) {
      const rel = b === a ? "同层互依赖" : "向上依赖";
      problems.push("分层：能力 " + from + "（第 " + a + " 层）" + rel + " " + to + "（第 " + b + " 层），见 " + file.slice(root.length + 1).replace(/\\/g, "/"));
    }
  }

  const wide = [...fanout].filter(([, n]) => n > FANOUT_TARGET).sort((x, y) => y[1] - x[1]);
  const notes = wide.length
    ? ["分层：扇出超过预算 " + FANOUT_TARGET + " 的能力（迁移目标，只报不拦）：" + wide.map(([c, n]) => c + " " + n).join("、")]
    : [];
  return { problems, notes };
}

module.exports = { layers, parseLayers };
