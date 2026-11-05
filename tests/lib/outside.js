/**
 * 项目外写检测：门禁跑完不许在项目外留下缓存/工具链写入（零豁免）。
 * 目的：借用系统工具链也不许写它的 home；监视根由 CARGO_HOME / RUSTUP_HOME 与家目录候选**派生**，
 *   不再假设只有 ~/.cargo 一个落点。
 * 这一层是运行期自检：开头快照，跑完再对，新增 / 删除 / 变化即硬失败。
 */
"use strict";
const fs = require("fs");
const os = require("os");
const path = require("path");

/** 项目内路径不算"项目外写"。 */
function inside(root, p) {
  const r = path.resolve(root).toLowerCase().replace(/\\/g, "/");
  const t = path.resolve(p).toLowerCase().replace(/\\/g, "/");
  return t === r || t.startsWith(r + "/");
}

/** 一个工具链 home 下值得盯的落点（目录或单文件）。 */
function homePoints(base, single) {
  return single.map((s) => path.join(base, s));
}

/**
 * 监视根：环境变量指到项目外的 home 必盯（那是本次运行真正会写的落点）；
 * 家目录候选无论环境变量是否覆盖都要盯（借用系统工具链时写的就是那里）。
 */
function watchRoots(root, env, home) {
  const e = env || process.env;
  const h = home || os.homedir();
  const roots = [];
  const add = (p) => {
    if (!p) return;
    const abs = path.resolve(p);
    if (inside(root, abs)) return; // 项目内写是允许的，不监视
    if (!roots.some((x) => x.toLowerCase() === abs.toLowerCase())) roots.push(abs);
  };
  if (e.CARGO_HOME) add(e.CARGO_HOME);
  if (e.RUSTUP_HOME) for (const p of homePoints(e.RUSTUP_HOME, ["settings.toml", "downloads", "tmp", "update-hashes"])) add(p);
  add(path.join(h, ".cargo"));
  for (const p of homePoints(path.join(h, ".rustup"), ["settings.toml", "downloads", "tmp", "update-hashes"])) add(p);
  add(path.join(h, ".npm"));
  return roots;
}

/** 递归记下目录树（目录/文件的 mtime 与大小）；封顶避免大目录拖慢门禁。 */
function snapshotTree(abs, acc, budget) {
  if (budget.n <= 0 || !fs.existsSync(abs)) return;
  let st;
  try { st = fs.lstatSync(abs); } catch (e) { return; }
  acc[abs] = st.isDirectory() ? "d:" + st.mtimeMs : "f:" + st.mtimeMs + ":" + st.size;
  budget.n--;
  if (!st.isDirectory()) return;
  let names;
  try { names = fs.readdirSync(abs); } catch (e) { return; }
  for (const n of names) {
    if (budget.n <= 0) break;
    snapshotTree(path.join(abs, n), acc, budget);
  }
}

function snapshot(root, env, home) {
  const acc = {};
  const budget = { n: 20000 };
  for (const r of watchRoots(root, env, home)) snapshotTree(r, acc, budget);
  return acc;
}

function diff(before, after) {
  const changed = [];
  for (const k of new Set(Object.keys(before).concat(Object.keys(after)))) {
    if (before[k] !== after[k]) changed.push(k + (before[k] === undefined ? "（新增）" : after[k] === undefined ? "（删除）" : "（变化）"));
  }
  return changed;
}

/** 自测：必须能分辨新增 / 删除 / 变化（门禁自己不可信就没有意义）。 */
function outsideSelfTest() {
  const bad = [];
  const eqo = (name, got, want) => { if (got !== want) bad.push("项目外写自测失败：" + name + "：实际 " + got + "，期望 " + want); };
  eqo("检出新增", diff({ a: "d:1" }, { a: "d:1", b: "f:2:3" }).length, 1);
  eqo("检出删除", diff({ a: "d:1", b: "f:2:3" }, { a: "d:1" }).length, 1);
  eqo("检出变化", diff({ a: "d:1" }, { a: "d:2" }).length, 1);
  eqo("无变化不误报", diff({ a: "d:1" }, { a: "d:1" }).length, 0);
  return bad;
}

/** 监视根派生自测：环境变量指到项目外要盯、指到项目内要放过、家目录候选要兜底、重复要去重。 */
function watchRootsSelfTest() {
  const bad = [];
  const root = path.resolve("/proj/solomni");
  const home = path.resolve("/home/u");
  const eqo = (name, got, want) => { if (JSON.stringify(got) !== JSON.stringify(want)) bad.push("监视根自测失败：" + name + "：实际 " + JSON.stringify(got) + "，期望 " + JSON.stringify(want)); };
  const derived = watchRoots(root, { CARGO_HOME: path.resolve("/proj/solomni/platform/cargo"), RUSTUP_HOME: path.resolve("/opt/rustup") }, home);
  eqo("环境变量指到项目内要放过", derived.some((p) => p.toLowerCase().startsWith(root.toLowerCase())), false);
  eqo("环境变量指到项目外要盯", derived.includes(path.resolve("/opt/rustup/downloads")), true);
  const fallback = watchRoots(root, {}, home);
  eqo("家目录候选兜底", fallback.includes(path.join(home, ".cargo")), true);
  eqo("家目录候选不重复", fallback.filter((p) => p === path.join(home, ".cargo")).length, 1);
  return bad;
}

/** 清掉上次崩溃残留在系统临时目录里的 solomni-*（pid 已不在 = 崩溃残留）。 */
function sweepStaleTemp() {
  const dir = os.tmpdir();
  const swept = [];
  let names;
  try { names = fs.readdirSync(dir); } catch (e) { return swept; }
  for (const n of names) {
    if (!/^solomni-/.test(n)) continue;
    const full = path.join(dir, n);
    const m = n.match(/-(\d+)$/);
    let stale = false;
    if (m) {
      try { process.kill(Number(m[1]), 0); stale = false; }
      catch (e) { stale = e && e.code === "EPERM" ? false : true; }
    } else {
      try { stale = Date.now() - fs.statSync(full).mtimeMs > 60 * 60 * 1000; } catch (e) { stale = true; }
    }
    if (!stale) continue;
    try { fs.rmSync(full, { recursive: true, force: true }); swept.push(n); } catch (e) { /* 清不掉就留着，报告里如实说 */ }
  }
  return swept;
}

module.exports = { watchRoots, snapshot, diff, outsideSelfTest, watchRootsSelfTest, sweepStaleTemp, inside };
