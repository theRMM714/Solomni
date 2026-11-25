/**
 * 结构审查·值边界门禁：跨能力只传值与端口，不传活对象（R15 / R16）。
 * 判据：① api*.rs 的公共函数与 trait 方法，参数里不得有 &mut——按**完整签名**判（可跨行）；
 *       豁免三条：&mut self、端口借用 &mut dyn Trait、泛型与回调边界里的 &mut（impl FnOnce(&mut T) 之类）——硬判；
 *       ② api*.rs 的值（pub struct）与它经 pub use 转出的同能力结构带内部可变（Mutex / RwLock / RefCell / Cell / UnsafeCell / Atomic），或字段是具名状态的可变借用 / 闭包（借端口 `&mut dyn Trait` 不算：那是把端口借进来）；
 *       ③ 跨能力的同形结构（字段名与类型全同）——②③ 只进 notes，等顶层编排定稿后转硬。
 *       ④ 契约唯一家与镜像禁令（H4）：同一个名字不得在两个契约面上各定义一次；
 *          呈现层/入口不得定义与契约同名的 pub 类型（那是复制 DTO，不是防腐）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { unitOfPath } = require("./unitpath.js");

const MUTABLE = /(Mutex<|RwLock<|RefCell<|Cell<|UnsafeCell<|Atomic(Usize|U64|U32|I64|Bool|U8))/;
const BORROW_OBJ = /(&'?\w*\s*mut\s|dyn Fn\b|Box<dyn Fn)/;
// 端口借用（`&mut dyn Trait`）：R16 允许——借进来的是端口，不是别人的活对象；闭包不在此列。
const PORT_BORROW = /&'?\w*\s*mut\s+(?:\(\s*)?dyn\s+[A-Z][A-Za-z0-9_]*/;

const rsFilesOf = (dir) => {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFilesOf(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

const capOf = (file) => unitOfPath(file);
const rel = (root, file) => path.relative(root, file).split(path.sep).join("/");
const shallow = (s) => s.replace(/\s+/g, " ").trim();

/** 取一个 struct 的字段（跳过注释行），返回 [{name, ty}]。 */
function fieldsOf(lines, start) {
  const out = [];
  for (let i = start + 1; i < Math.min(start + 90, lines.length); i++) {
    const line = lines[i];
    if (line.trim() === "}") break;
    if (/^\s*\/\//.test(line)) continue;
    const m = line.match(/^\s+(?:pub )?([a-z_][a-z0-9_]*):\s*(.+?),\s*$/);
    if (m) out.push({ name: m[1], ty: shallow(m[2]) });
  }
  return out;
}

/** 收集能力源码里的全部 pub struct（按名字建索引，用来做传递可达）。 */
function structIndex(files) {
  const index = new Map();
  for (const file of files) {
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^pub struct ([A-Za-z_][A-Za-z0-9_]*)/);
      if (!m) continue;
      if (!index.has(m[1])) index.set(m[1], []);
      index.get(m[1]).push({ file, fields: fieldsOf(lines, i) });
    }
  }
  return index;
}

/** api 面 `pub use` 转出的类型名：它们是跨能力可见的契约面，和 api 里直接声明的一样受判。 */
function reexported(lines) {
  const out = [];
  // `pub use` 可能跨多行（花括号折行），先拼成一条语句再取名字。
  const stmts = [];
  for (let i = 0; i < lines.length; i++) {
    if (!/^pub use\s+/.test(lines[i])) continue;
    let stmt = lines[i];
    for (let j = i + 1; j < lines.length && !/;\s*$/.test(stmt); j++) stmt += lines[j];
    stmts.push(stmt);
  }
  for (const stmt of stmts) {
    const m = stmt.match(/^pub use\s+([\s\S]+);\s*$/);
    if (!m) continue;
    const brace = m[1].match(/\{([^}]*)\}/);
    const names = brace ? brace[1].split(",") : [m[1]];
    for (const raw of names) {
      const nm = raw.trim().replace(/\s+as\s+.*$/, "");
      const last = nm.split("::").pop();
      if (/^[A-Z][A-Za-z0-9_]*$/.test(last)) out.push(last);
    }
  }
  return out;
}

/** 顺着字段类型找内部可变，返回命中的「类型.字段」；只走唯一命名的类型，避免同名歧义。 */
function reachMutable(index, name, depth, seen) {
  if (depth <= 0 || seen.has(name)) return null;
  const defs = index.get(name);
  if (!defs || defs.length !== 1) return null;
  seen.add(name);
  for (const f of defs[0].fields) {
    if (MUTABLE.test(f.ty)) return name + "." + f.name;
    for (const n of f.ty.match(/\b[A-Z][A-Za-z0-9_]*\b/g) || []) {
      const hit = reachMutable(index, n, depth - 1, seen);
      if (hit) return hit;
    }
  }
  return null;
}

/** 参数表里第一个出境的可变借用；豁免 &mut self、端口借用、泛型与回调边界。 */
function mutableParam(params) {
  const re = /&mut\s+/g;
  let m;
  while ((m = re.exec(params))) {
    const after = params.slice(m.index + m[0].length);
    // &mut self：方法自己的可变接收者，不是出境的状态。
    if (/^self\b/.test(after)) continue;
    // &mut dyn Trait / &mut impl Trait：借进来的是端口（R16 允许窄端口）。
    if (/^(dyn|impl)\s+([A-Z]|crate::)/.test(after)) continue;
    // 泛型与回调边界：impl FnOnce(&mut T)、fn(&mut T)、Box<dyn Fn(&mut T)>。
    if (/\b(FnOnce|FnMut|Fn|fn)\s*\(\s*$/.test(params.slice(0, m.index))) continue;
    return shallow(params.slice(Math.max(0, m.index - 40), m.index + 60));
  }
  return null;
}

/** 公共签名里的可变借用：判 api 面**完整的**参数表（跨行也看得见），trait 方法与 pub fn 都算。 */
function borrowedProblems(root, file) {
  const problems = [];
  const text = fs.readFileSync(file, "utf8");
  const lines = text.split(/\r?\n/);
  const starts = [];
  let at = 0;
  for (const line of lines) {
    starts.push(at);
    at += line.length + 1;
  }
  let inPubTrait = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^pub trait /.test(line)) inPubTrait = true;
    else if (/^\}/.test(line)) inPubTrait = false;
    if (!/^\s*(pub )?fn /.test(line)) continue;
    if (!inPubTrait && !/^\s*pub fn /.test(line)) continue;
    const fnAt = line.search(/\bfn\s+[A-Za-z_][A-Za-z0-9_]*/);
    if (fnAt < 0) continue;
    const open = text.indexOf("(", starts[i] + fnAt);
    if (open < 0) continue;
    let depth = 0;
    let end = -1;
    for (let k = open; k < text.length; k++) {
      if (text[k] === "(") depth++;
      else if (text[k] === ")") {
        depth--;
        if (depth === 0) {
          end = k;
          break;
        }
      }
    }
    if (end < 0) continue;
    const hit = mutableParam(text.slice(open + 1, end));
    if (!hit) continue;
    problems.push("值边界：" + rel(root, file) + ":" + (i + 1) + " 的公共签名传了可变借用（R16）：" + hit.slice(0, 80));
  }
  return problems;
}

/**
 * ④ 契约唯一家与镜像禁令（H4）：一个名字只属于一个契约面。
 *   契约面 = `src/kernel/contracts/*.rs` 与 `src/units/<unit>/api{.rs,/}`；
 *   同一个名字在两个契约面上各出现一次，或呈现层/入口再造一份同名的 pub 类型，都是复制契约。
 *   防腐层（同义模型 + 翻译）只准在 `units/<unit>/detail`、呈现层的屏幕装配、入口适配处，且不得同名镜像。
 */
function contractProblems(root) {
  const files = rsFilesOf(path.join(root, "src")).filter((f) => !f.includes(path.sep + "tests" + path.sep));
  const isContract = (r) => /^src\/kernel\/contracts\//.test(r) || /\/api\.rs$/.test(r) || /\/api\//.test(r);
  const isAdapter = (r) => /^src\/(presentation|entry)\//.test(r);
  const defs = new Map();
  for (const file of files) {
    const r = rel(root, file);
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^\s*pub\s+(?:struct|enum|type)\s+([A-Za-z_][A-Za-z0-9_]*)/);
      if (!m) continue;
      if (!defs.has(m[1])) defs.set(m[1], []);
      defs.get(m[1]).push({ at: r + ":" + (i + 1), contract: isContract(r), adapter: isAdapter(r) });
    }
  }
  const problems = [];
  for (const [name, list] of defs) {
    const contracts = list.filter((d) => d.contract);
    if (!contracts.length) continue;
    if (new Set(contracts.map((d) => d.at.split(":")[0])).size > 1) {
      problems.push("契约唯一家：" + name + " 在两个契约面上各定义一次（H4）：" + contracts.map((d) => d.at).join("、"));
      continue;
    }
    const mirrors = list.filter((d) => !d.contract && d.adapter);
    if (mirrors.length) {
      problems.push(
        "契约镜像：" + name + " 已有契约定义（" + contracts[0].at + "），适配层又造了一份（H4）：" + mirrors.map((d) => d.at).join("、"),
      );
    }
  }
  return problems;
}

/** 值边界（api 面）与同形重复（全能力面）：返回 {problems, notes}。 */
function values(root) {
  const dir = path.join(root, "src", "units");
  const files = rsFilesOf(dir).filter((f) => !f.includes(path.sep + "tests" + path.sep));
  const api = files.filter((f) => /(api\.rs|api\/.*\.rs)$/.test(f));
  const index = structIndex(files);
  const problems = [];
  const notes = [];
  const mutable = [];
  const borrows = [];
  const reported = new Set();
  // 一个跨能力可见的结构：字段带锁 / 带可变借用 / 顺着字段可达锁，都算活对象。
  const judge = (cap, name, fields, at) => {
    const key = cap + "::" + name;
    if (reported.has(key)) return;
    const locked = fields.filter((f) => MUTABLE.test(f.ty));
    const borrowed = fields.filter(
      (f) => BORROW_OBJ.test(f.ty) && !(PORT_BORROW.test(f.ty) && !/dyn\s+Fn/.test(f.ty)),
    );
    if (locked.length) mutable.push(name + "（" + at + "，字段 " + locked.map((f) => f.name).join("、") + "）");
    if (borrowed.length) borrows.push(name + "（" + at + "，字段 " + borrowed.map((f) => f.name).join("、") + "）");
    if (locked.length || borrowed.length) {
      reported.add(key);
      return;
    }
    for (const f of fields) {
      for (const n of f.ty.match(/\b[A-Z][A-Za-z0-9_]*\b/g) || []) {
        const hit = reachMutable(index, n, 4, new Set([name]));
        if (hit) {
          mutable.push(name + "（" + at + "，经 " + n + " 到 " + hit + "）");
          reported.add(key);
          return;
        }
      }
    }
  };
  // 定义位置：只用于回执里指路（同一个名字在能力内唯一才判）。
  const defAt = (defs, name) => {
    const file = defs[0].file;
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].match(new RegExp("^pub struct " + name + "\\b"))) return rel(root, file) + ":" + (i + 1);
    }
    return rel(root, file);
  };
  for (const file of api) {
    problems.push(...borrowedProblems(root, file));
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^pub struct ([A-Za-z_][A-Za-z0-9_]*)/);
      if (!m) continue;
      judge(capOf(file), m[1], fieldsOf(lines, i), rel(root, file) + ":" + (i + 1));
    }
    for (const name of reexported(lines)) {
      const defs = index.get(name);
      if (!defs || defs.length !== 1) continue;
      const cap = capOf(file);
      if (capOf(defs[0].file) !== cap) continue;
      judge(cap, name, defs[0].fields, defAt(defs, name) + "（经 " + rel(root, file) + " 转出）");
    }
  }
  if (mutable.length) notes.push("值边界：api 的值带内部可变或可达内部可变，跨能力传递时它就是活对象（R16 迁移清单）：" + mutable.join("、"));
  if (borrows.length) notes.push("值边界：api 的值里塞了可变借用或闭包，等于把对象当参数传（R16 迁移清单）：" + borrows.join("、"));
  const everyStruct = [];
  const same = new Map();
  for (const file of files) {
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^pub struct ([A-Za-z_][A-Za-z0-9_]*)/);
      if (m) everyStruct.push({ cap: capOf(file), name: m[1], fields: fieldsOf(lines, i) });
    }
  }
  for (const s of everyStruct) {
    if (s.fields.length < 2) continue;
    const key = s.fields.map((f) => f.name + ":" + f.ty).sort().join("|");
    if (!same.has(key)) same.set(key, []);
    same.get(key).push(s);
  }
  for (const group of same.values()) {
    if (new Set(group.map((s) => s.cap)).size < 2) continue;
    const names = [...new Set(group.map((s) => (s.cap ? s.cap + "::" : "") + s.name))];
    notes.push("值边界：跨能力同形结构（R15 迁移清单）：" + names.join(" = "));
  }
  problems.push(...contractProblems(root));
  return { problems, notes };
}

module.exports = { values };