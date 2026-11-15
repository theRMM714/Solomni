/**
 * 结构审查·值边界门禁：跨能力只传值与端口，不传活对象（R15 / R16）。
 * 判据：① api*.rs 的公共函数与 trait 方法，参数里不得有 &mut（&mut self 与流式回调 &mut dyn FnMut 除外）——硬判；
 *       ② api*.rs 的值（pub struct）带内部可变（Mutex / RwLock / RefCell / Cell / UnsafeCell / Atomic），或字段是可变借用 / 闭包；
 *       ③ 跨能力的同形结构（字段名与类型全同）——②③ 只进 notes，等 conductor 消失后转硬。
 */
"use strict";
const fs = require("fs");
const path = require("path");

const MUTABLE = /(Mutex<|RwLock<|RefCell<|Cell<|UnsafeCell<|Atomic(Usize|U64|U32|I64|Bool|U8))/;
const BORROW_OBJ = /(&'?\w*\s*mut\s|dyn Fn\b|Box<dyn Fn)/;
const STREAM_CB = /&'?\w*\s*mut dyn FnMut/;

const rsFilesOf = (dir) => {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFilesOf(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

const capOf = (file) => {
  const parts = file.split(path.sep);
  const at = parts.indexOf("capabilities");
  return at >= 0 ? parts[at + 1] : "";
};
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

/** 公共签名里的可变借用：只判 api 面，trait 方法与 pub fn 都算。 */
function borrowedProblems(root, file) {
  const problems = [];
  const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
  let inPubTrait = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^pub trait /.test(line)) inPubTrait = true;
    else if (/^\}/.test(line)) inPubTrait = false;
    const isFn = /^\s*(pub )?fn /.test(line);
    if (!isFn) continue;
    if (!/&mut /.test(line)) continue;
    if (/&mut self\b/.test(line)) continue;
    if (STREAM_CB.test(line)) continue;
    if (!inPubTrait && !/^\s*pub fn /.test(line)) continue;
    problems.push("值边界：" + rel(root, file) + ":" + (i + 1) + " 的公共签名传了可变借用（R16）：" + shallow(line).slice(0, 80));
  }
  return problems;
}

/** 值边界（api 面）与同形重复（全能力面）：返回 {problems, notes}。 */
function values(root) {
  const dir = path.join(root, "src", "capabilities");
  const files = rsFilesOf(dir).filter((f) => !f.includes(path.sep + "tests" + path.sep));
  const api = files.filter((f) => /(api\.rs|api\/.*\.rs)$/.test(f));
  const index = structIndex(files);
  const problems = [];
  const notes = [];
  const mutable = [];
  const borrows = [];
  const apiStructs = [];
  for (const file of api) {
    problems.push(...borrowedProblems(root, file));
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^pub struct ([A-Za-z_][A-Za-z0-9_]*)/);
      if (!m) continue;
      const fields = fieldsOf(lines, i);
      const at = rel(root, file) + ":" + (i + 1);
      apiStructs.push({ cap: capOf(file), name: m[1], fields });
      const locked = fields.filter((f) => MUTABLE.test(f.ty));
      if (locked.length) mutable.push(m[1] + "（" + at + "，字段 " + locked.map((f) => f.name).join("、") + "）");
      const borrowed = fields.filter((f) => BORROW_OBJ.test(f.ty));
      if (borrowed.length) borrows.push(m[1] + "（" + at + "，字段 " + borrowed.map((f) => f.name).join("、") + "）");
      if (locked.length || borrowed.length) continue;
      for (const f of fields) {
        for (const n of f.ty.match(/\b[A-Z][A-Za-z0-9_]*\b/g) || []) {
          const hit = reachMutable(index, n, 4, new Set([m[1]]));
          if (hit) { mutable.push(m[1] + "（" + at + "，经 " + n + " 到 " + hit + "）"); break; }
        }
        if (mutable.length && mutable[mutable.length - 1].startsWith(m[1] + "（")) break;
      }
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
  return { problems, notes };
}

module.exports = { values };
