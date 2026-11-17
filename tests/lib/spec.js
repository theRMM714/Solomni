/**
 * 用例声明（tests/specs/*.yaml）的解析与校验：**测试清单的唯一声明处**。
 * 为什么要有它：用例散在 Rust 文件与 e2e 里时，"某单元有哪些用例、某个用例属于谁"只能靠文件名猜；
 * 重构搬目录就全量失真。声明与实现分开后，搬实现只动适配层，声明仍指向行为。
 *
 * 语法是**受限子集**（不是通用 YAML）：只认 unit / title / cases，以及每个 case 的 id / tier / note。
 * 认不出的语法一律报错（带行号）——静默忽略会让"声明了但没生效"看起来全绿。
 */
"use strict";

const fs = require("fs");
const path = require("path");

const KINDS = ["cargo", "platform", "e2e"];
const TIERS = { cargo: ["T1", "T2", "T3"], platform: ["T4"], e2e: ["T5"] };

/** 去掉行尾注释与两侧空白；引号包住的值去引号（注释符在引号里不算注释）。 */
function valueOf(raw) {
  let s = raw.trim();
  const q = s.match(/^(['"])(.*)\1$/);
  if (q) return q[2];
  const hash = s.indexOf(" #");
  if (hash >= 0) s = s.slice(0, hash).trim();
  return s;
}

/** 解析一份 spec 文本；返回 { unit, title, cases } 或抛出带行号的错误。 */
function parseSpec(text, file) {
  const out = { unit: null, title: null, cases: [] };
  const lines = String(text).split(/\r?\n/);
  let inCases = false;
  let cur = null;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const t = raw.trim();
    if (!t || t.startsWith("#")) continue;
    const at = () => (file ? file + ":" + (i + 1) : "第 " + (i + 1) + " 行");
    const indent = raw.length - raw.replace(/^\s+/, "").length;
    if (indent === 0) {
      const m = t.match(/^(unit|title|cases):\s*(.*)$/);
      if (!m) throw new Error(at() + " 无法解析：" + t + "（只认 unit / title / cases）");
      if (m[1] === "cases") {
        if (m[2]) throw new Error(at() + " cases 只能换行写列表");
        inCases = true;
        continue;
      }
      if (m[1] === "unit") out.unit = valueOf(m[2]);
      else out.title = valueOf(m[2]);
      if (!valueOf(m[2])) throw new Error(at() + " " + m[1] + " 不能为空");
      continue;
    }
    if (!inCases) throw new Error(at() + " 缩进行只能出现在 cases: 之后：" + t);
    if (indent === 2 && t.startsWith("- ")) {
      cur = { id: null, tier: null, note: "" };
      out.cases.push(cur);
      const kv = t.slice(2).match(/^([a-z]+):\s*(.*)$/);
      if (!kv || kv[1] !== "id") throw new Error(at() + " 列表项的第一行必须是 - id: …：" + t);
      cur.id = valueOf(kv[2]);
      continue;
    }
    if (indent === 4 && cur) {
      const kv = t.match(/^([a-z]+):\s*(.*)$/);
      if (!kv) throw new Error(at() + " 无法解析：" + t);
      if (kv[1] === "tier") cur.tier = valueOf(kv[2]);
      else if (kv[1] === "note") cur.note = valueOf(kv[2]);
      else throw new Error(at() + " 未知字段：" + kv[1] + "（只认 tier / note）");
      continue;
    }
    throw new Error(at() + " 缩进不对（列表项 2 格、字段 4 格）：" + t);
  }
  if (!out.unit) throw new Error((file || "spec") + " 缺 unit:");
  for (const [i, c] of out.cases.entries()) {
    if (!c.id) throw new Error((file || "spec") + " 第 " + (i + 1) + " 个用例缺 id");
    if (!c.tier) throw new Error((file || "spec") + " 用例 " + c.id + " 缺 tier");
  }
  return out;
}

/** 用例 id → { kind, target }；形状不对返回 null。 */
function splitCaseId(id) {
  const m = String(id || "").match(/^([a-z0-9]+)\/(\S+)$/);
  if (!m || !KINDS.includes(m[1])) return null;
  return { kind: m[1], target: m[2] };
}

/**
 * cargo 用例的 target 是模块路径（如 tests::api::session）：落到 src/tests 下的文件或目录。
 * 只为**静态可解析**：模块路径写得对但文件被搬走，这里立刻能报出来（用例不会静默消失）。
 */
function cargoTargetExists(root, target) {
  const parts = String(target).split("::");
  if (parts[0] !== "tests" || parts.length < 2) return false;
  const rel = parts.slice(1);
  const base = path.join(root, "src", "tests");
  // 依次尝试：tests/a/b.rs、tests/a/b/mod.rs、tests/a/b/（目录里有任意 .rs）
  const walk = (dir, i) => {
    const p = path.join(dir, rel[i]);
    const last = i === rel.length - 1;
    if (last) return fs.existsSync(p + ".rs") || fs.existsSync(path.join(p, "mod.rs"));
    if (!fs.existsSync(p) || !fs.statSync(p).isDirectory()) return false;
    return walk(p, i + 1);
  };
  return walk(base, 0);
}

/** platform 用例的 target 是 tests/<name>/ 这个 Rust 测试目标。 */
function platformTargetExists(root, target) {
  return fs.existsSync(path.join(root, "tests", target, "main.rs")) || fs.existsSync(path.join(root, "tests", target));
}

/** 读全部 spec（按文件名排序），返回 { specs, problems }；problems 是硬错误（门禁判红）。 */
function loadSpecs(root) {
  const dir = path.join(root, "tests", "specs");
  const problems = [];
  const specs = [];
  if (!fs.existsSync(dir)) return { specs, problems: ["缺 tests/specs/：用例声明是测试清单的唯一声明处"] };
  for (const name of fs.readdirSync(dir).sort()) {
    if (!name.endsWith(".yaml")) {
      problems.push("tests/specs/" + name + "：只认 .yaml");
      continue;
    }
    let parsed;
    try {
      parsed = parseSpec(fs.readFileSync(path.join(dir, name), "utf8"), "tests/specs/" + name);
    } catch (e) {
      problems.push(e.message);
      continue;
    }
    const stem = name.replace(/\.yaml$/, "");
    if (parsed.unit !== stem) problems.push("tests/specs/" + name + "：unit: " + parsed.unit + " 与文件名不一致（一个单元一份，文件名就是单元名）");
    specs.push(Object.assign({ file: "tests/specs/" + name }, parsed));
  }
  return { specs, problems };
}

module.exports = { KINDS, TIERS, parseSpec, splitCaseId, cargoTargetExists, platformTargetExists, loadSpecs };
