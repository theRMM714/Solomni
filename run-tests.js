#!/usr/bin/env node
/**
 * 测试总入口（见 TESTING.md）：先问本机事实（solomni --doctor），再逐目标点名跑，最后汇总四态。
 * T0 质量门禁全部是**零容忍硬失败**：编译、结构审查、格式、clippy、编译告警、依赖重复——
 * 没有存量基线（见 docs/testing/quality-isolation.md）。
 * 直接跑，或经 node start.js -test（后者会先把项目内工具链环境备好，再调本脚本）。
 */
"use strict";
const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const OS_KEY = IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const PROFILE = process.argv.includes("--release") ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const PLATFORM_TARGETS = ["cross-platform", "windows", "linux", "macos"];
// 真机围栏测试（会改本机状态：建 AppContainer profile、写目录 ACL）默认不跑，必须显式开启。
const FENCE_LIVE = process.argv.includes("--fence-live") || process.env.SOLOMNI_FENCE_LIVE === "1";
const REPORT = path.join(ROOT, "target", "test-report.json");
// 缺口账：唯一真相是这些文件。平台账决定 TEST-REPORT-ACCEPTED；全局账是长期目标（每条都进报告）。
const GAP_FILES = [
  path.join(ROOT, "tests", "gaps.yaml"),
  path.join(ROOT, "tests", "cross-platform", "gaps.yaml"),
  ...[IS_WIN ? "windows" : OS_KEY, "windows", "linux", "macos"].map((p) => path.join(ROOT, "tests", p, "gaps.yaml")),
];

function buildEnv() {
  // start.js -test 会传好现成的环境；直接跑时尽力指向项目内工具链（不碰系统安装）。
  const e = Object.assign({}, process.env);
  // 把开关传给测试与产品：默认"不写本机状态"，只有 --fence-live 才允许。
  e.SOLOMNI_FENCE_LIVE = FENCE_LIVE ? "1" : "0";
  e.SOLOMNI_FENCE_WRITE = FENCE_LIVE ? "1" : "0";
  const osDir = path.join(ROOT, "platform", IS_WIN ? "windows" : "linux");
  // 项目内工具链存在就用它（本地收敛原则）；不存在（例如 CI runner）就用环境里现成的。
  const localCargo = path.join(osDir, "cargo");
  const localRustup = path.join(osDir, "rustup");
  if (!e.CARGO_HOME && fs.existsSync(localCargo)) e.CARGO_HOME = localCargo;
  if (!e.RUSTUP_HOME && fs.existsSync(localRustup)) e.RUSTUP_HOME = localRustup;
  const mingw = path.join(ROOT, ".tools", "mingw64", "bin");
  const cargoBin = path.join(osDir, "cargo", "bin");
  const KEY = Object.keys(e).find((k) => k.toUpperCase() === "PATH") || "PATH";
  const front = [cargoBin, IS_WIN ? mingw : null].filter((p) => p && fs.existsSync(p));
  if (front.length) e[KEY] = front.join(path.delimiter) + path.delimiter + (e[KEY] || "");
  return e;
}

let stepNo = 0;
/// 每步的起跑时刻：报告里带 ms，才能看出"哪一步慢"（真机上 Windows 的 L1 比 Linux 慢两个数量级，就是靠这个定位）
let stepStart = Date.now();
function announce(label) {
  stepNo++;
  stepStart = Date.now();
  process.stdout.write("[" + stepNo + "] " + label + " ... ");
}
function announceDone(status, detail) {
  console.log(status + (detail ? "（" + detail + "）" : "") + "  [" + ((Date.now() - stepStart) / 1000).toFixed(1) + "s]");
}
function sh(cmd, args) {
  // 输出走文件而不是管道：受限环境里"用管道抓子进程输出"会 EPERM；落成日志还顺带留了档案。
  const logDir = path.join(ROOT, "target", "test-logs");
  fs.mkdirSync(logDir, { recursive: true });
  const slug = (s) => String(s).replace(/[^\w.-]+/g, "_");
  const logFile = path.join(logDir, slug(path.basename(cmd)) + "-" + args.map(slug).join("_") + ".log");
  const fd = fs.openSync(logFile, "w");
  const r = spawnSync(cmd, args, { cwd: ROOT, env: buildEnv(), stdio: ["ignore", fd, fd] });
  fs.closeSync(fd);
  const out = fs.readFileSync(logFile, "utf8");
  return { code: r.error ? -1 : r.status, out: out, error: r.error ? String(r.error.message) : null, log: path.relative(ROOT, logFile) };
}

/** 汇总 cargo 的 test result 行：一次运行可能有多条（多目标/多测试）。 */
function cargoCounts(out) {
  let passed = 0, failed = 0, resultLines = 0;
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed/);
    if (m) { resultLines++; passed += Number(m[2]); failed += Number(m[3]); }
  }
  return { passed, failed, resultLines };
}

function skipsIn(out) {
  return out.split(/\r?\n/).filter((l) => l.includes("[探针]")).map((l) => l.trim());
}

/** 读该平台（含跨平台层）的缺口账：条目存在 = 这条测试还没有。 */
function gapLedgers() {
  const files = [path.join(ROOT, "tests", "cross-platform", "gaps.yaml"), path.join(ROOT, "tests", OS_KEY, "gaps.yaml")];
  const ids = [];
  for (const f of files) {
    if (!fs.existsSync(f)) continue;
    for (const line of fs.readFileSync(f, "utf8").split(/\r?\n/)) {
      const m = line.match(/^\s*-\s*id:\s*(\S+)/);
      if (m) ids.push(m[1]);
    }
  }
  return ids;
}

/** 全局缺口账（tests/gaps.yaml）：长期目标；每条都进报告，但不影响平台的 ACCEPTED 判定。 */
function globalGaps() {
  const f = path.join(ROOT, "tests", "gaps.yaml");
  if (!fs.existsSync(f)) return [];
  const ids = [];
  for (const line of fs.readFileSync(f, "utf8").split(/\r?\n/)) {
    const m = line.match(/^\s*-\s*id:\s*(\S+)/);
    if (m) ids.push(m[1]);
  }
  return ids;
}

// ---------- T0：质量检查的解析（路径归一 + 四项的原始输出解析） ----------

// 词法归一：统一分隔符、去掉 \\?\ 扩展前缀、收掉 '.' 与 '..' 段。
// 为什么必须收 '..'：同一个文件可以被多个测试目标用 #[path] 引用，rustfmt 会按"目标根 + 相对路径"
// 报出来（unix 上是 tests/cross-platform/../helpers/probe.rs），Windows 上则报规范化后的路径。
// 归一之后三平台得到同一份名单，同一个文件不会被算成两个。
const normPath = (s) => {
  const parts = s.replace(/\\/g, "/").replace(/^\/\/\?\//, "").split("/");
  const out = [];
  for (const seg of parts) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      out.pop();
      continue;
    }
    out.push(seg);
  }
  return out.join("/");
};
const ROOT_NORM = normPath(ROOT) + "/";

/** fmt --check：有偏差的文件集合（仓库相对路径、/ 分隔）。 */
function fmtDeviations(out) {
  const files = new Set();
  // 先剥 ANSI 颜色转义：Windows 上 rustfmt 会带颜色输出，而下面按**行首**锚定，
  // 带转义前缀的行会被整条漏掉（实测：一条真正的格式偏差就是这样从 Windows 门禁眼皮底下溜过去的）。
  for (const line of out.replace(/\u001b\[[0-9;]*m/g, "").split(/\r?\n/)) {
    const m = line.match(/^Diff in (.+?):\d+:\s*$/);
    if (!m) continue;
    let p = normPath(m[1]);
    if (p.startsWith(ROOT_NORM)) p = p.slice(ROOT_NORM.length);
    files.add(p);
  }
  return files;
}

/** clippy：按 lint 名计数（每条诊断恰好一个 rust-clippy 文档锚点）。 */
function clippyLints(out) {
  const counts = {};
  for (const m of out.matchAll(/rust-clippy\/[^#\s]*#([a-z0-9_-]+)/g)) {
    const name = m[1].replace(/-/g, "_");
    counts[name] = (counts[name] || 0) + 1;
  }
  return counts;
}

/** cargo check 的 rustc 层告警数（不把"generated N warnings"这类汇总行算进去）。 */
function checkWarnings(out) {
  let n = 0;
  for (const line of out.split(/\r?\n/)) {
    if (/^warning: /.test(line) && !/generated \d+ warning/.test(line)) n++;
  }
  return n;
}

/** cargo tree --duplicates 报出的重复 crate 名（列首的 "name vX.Y.Z"）。 */
function duplicateCrates(out) {
  const names = new Set();
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^([A-Za-z0-9_.-]+) v\d/);
    if (m) names.add(m[1]);
  }
  return [...names].sort();
}

/** 工具缺失 = env-skip（不静默算过）。 */
function toolAvailable(sub) {
  return sh("cargo", [sub, "--version"]).code === 0;
}

/** 结构审查（纯文件分析，不起进程）：目标登记、孤儿测试文件、缺口账格式。 */
function structuralAudit() {
  const problems = [];
  const toml = fs.readFileSync(path.join(ROOT, "Cargo.toml"), "utf8");
  const targets = [];
  for (const block of toml.split(/\[\[test\]\]/).slice(1)) {
    const name = (block.match(/^\s*name\s*=\s*"([^"]+)"/m) || [])[1];
    const p = (block.match(/^\s*path\s*=\s*"([^"]+)"/m) || [])[1];
    if (!name || !p) { problems.push("Cargo.toml 有 [[test]] 缺 name 或 path"); continue; }
    if (!fs.existsSync(path.join(ROOT, p))) problems.push("测试目标 " + name + " 的入口不存在：" + p);
    targets.push({ name, entry: path.join(ROOT, p) });
  }
  if (!targets.length) problems.push("Cargo.toml 没有登记任何 [[test]] 目标");

  const reachable = new Set();
  const walk = (file) => {
    const abs = path.resolve(file);
    if (reachable.has(abs) || !fs.existsSync(abs)) return;
    reachable.add(abs);
    const dir = path.dirname(abs);
    const re = /(?:#\[path\s*=\s*"([^"]+)"\]\s*)?(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
    let m;
    while ((m = re.exec(fs.readFileSync(abs, "utf8")))) {
      const mod = m[2];
      if (!mod) continue;
      const cands = m[1]
        ? [path.resolve(dir, m[1])]
        : [path.join(dir, mod + ".rs"), path.join(dir, mod, "mod.rs")];
      for (const c of cands) if (fs.existsSync(c)) { walk(c); break; }
    }
  };
  for (const t of targets) walk(t.entry);

  const allTestFiles = [];
  const collect = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) collect(p);
      else if (e.name.endsWith(".rs")) allTestFiles.push(p);
    }
  };
  collect(path.join(ROOT, "tests"));
  for (const f of allTestFiles) {
    if (!reachable.has(path.resolve(f))) {
      problems.push("孤儿测试文件（没有任何测试目标引用它）：" + path.relative(ROOT, f).replace(/\\/g, "/"));
    }
  }

  const rel = (f) => path.relative(ROOT, f).replace(/\\/g, "/");
  for (const f of GAP_FILES) {
    if (!fs.existsSync(f)) { problems.push("缺缺口账：" + rel(f)); continue; }
    const blocks = fs.readFileSync(f, "utf8").split(/^\s*-\s*id:/m).slice(1);
    blocks.forEach((b, i) => {
      for (const key of ["scope", "level", "why", "how", "accept", "blocked_by"]) {
        if (!new RegExp("^\\s+" + key + ":", "m").test(b)) problems.push(rel(f) + " 第 " + (i + 1) + " 条缺口缺 " + key);
      }
    });
  }

  // 文档分层（AGENTS.md「文档分层与同步」）：门户引用 docs/ 下的细则，细则引用彼此——
  // 两边都要真实存在。只查引用不查正文，避免把文档写法变成门禁。
  // **相对解析**：链接按所在文件的目录解析（门户在根、细则在 docs/<领域>/），所以 docs/ 内部写错的同级引用也会被抓到。
  const mdLinks = (text) => [...text.matchAll(/\]\(([^)#\s]+\.md)(?:#[^)]*)?\)/g)].map((m) => m[1]);
  const checkDocLinks = (absFile, label) => {
    const dir = path.dirname(absFile);
    for (const ref of mdLinks(fs.readFileSync(absFile, "utf8"))) {
      if (/^[a-z]+:/i.test(ref)) continue; // 外链不查
      if (!fs.existsSync(path.resolve(dir, ref))) problems.push(label + " 引用的文档不存在：" + ref);
    }
  };
  // 结构审查：**跨平台字面量完整性**——平台专属代码本地不编译（unix 的 #[cfg] 在 Windows 上不存在，
  // 反之亦然），漏字段这类错只在别的平台炸。这里按文本检查：每个 FenceSpec 字面量都要写全字段
  // （`ro` 与 `rw` 同层），本地就能抓住 CI 才会暴露的那类问题。
  const checkFenceSpecLiterals = (dir) => {
    const walk = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) { walk(p); continue; }
        if (!e.name.endsWith(".rs")) continue;
        const text = fs.readFileSync(p, "utf8");
        const lines = text.split(/\r?\n/);
        lines.forEach((line, i) => {
          // 只认**字面量**：排除 `struct FenceSpec {` / `impl FenceSpec {` 与函数签名（含 `->`）。
          const t = line.trim();
          if (!t.endsWith("FenceSpec {") || t.includes("->")) return;
          if (/^(pub\s+)?(struct|enum|impl|fn)\b/.test(t)) return;
          // 字面量以单独一行 `}` 或 `};` 收尾：在这之前必须出现 ro。
          let end = i + 1;
          while (end < lines.length && !/^\s*\};?\s*$/.test(lines[end])) end++;
          const body = lines.slice(i + 1, end).join("\n");
          if (!/\bro:/.test(body)) {
            problems.push(rel(p) + ":" + (i + 1) + " 的 FenceSpec 字面量缺 ro（平台专属代码本地不编译，CI 才会炸）");
          }
        });
      }
    };
    walk(dir);
  };
  checkFenceSpecLiterals(path.join(ROOT, "src"));

  // 平台专属文件本地不编译（tests/<平台>/main.rs 首行是 #![cfg(target_os = …)]）：
  // 常量名写串了（macOS 探针里写 Linux 的标记）只在那个平台上炸。按文本挡住这一对。
  const crossPlatformMarks = [
    ["tests/macos", "RULES_REJECTED_MARK", "Linux 的标记出现在 macOS 探针里"],
    ["tests/linux", "PROFILE_REJECTED_MARK", "macOS 的标记出现在 Linux 探针里"],
  ];
  for (const [dir, needle, why] of crossPlatformMarks) {
    const abs = path.join(ROOT, dir);
    if (!fs.existsSync(abs)) continue;
    const walkMarks = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) { walkMarks(p); continue; }
        if (!e.name.endsWith(".rs")) continue;
        fs.readFileSync(p, "utf8")
          .split(/\r?\n/)
          .forEach((line, i) => {
            if (line.includes(needle) && !line.trim().startsWith("//")) {
              problems.push(rel(p) + ":" + (i + 1) + " " + why + "（本地不编译，CI 才会炸）");
            }
          });
      }
    };
    walkMarks(abs);
  }

  const portalFiles = ["README.md", "README_EN.md", "AGENTS.md", "ARCHITECTURE.md", "PRODUCT.md", "MODULE_SPEC.md", "RUNTIME_SPEC.md", "REGISTRY_SPEC.md", "TESTING.md"];
  for (const p of portalFiles) {
    const abs = path.join(ROOT, p);
    if (!fs.existsSync(abs)) { problems.push("缺门户文档：" + p); continue; }
    checkDocLinks(abs, p);
  }
  const docsDir = path.join(ROOT, "docs");
  if (fs.existsSync(docsDir)) {
    const walkDocs = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) { walkDocs(p); continue; }
        if (!e.name.endsWith(".md")) continue;
        checkDocLinks(p, rel(p));
      }
    };
    walkDocs(docsDir);
  }

  // ---------- 依赖方向门禁 ----------
  // 权威：AGENTS.md「核心约束」（按业务功能垂直切分、业务之间通过 API 契约协作）与
  // ARCHITECTURE.md §一「分层与依赖方向」；目标分区与销账口径见 docs/architecture/refactor-plan.md §一 / §四。
  // 三条规则：① 业务/机制层不得反向依赖 adapters / presentation；
  //          ② presentation 只经入站能力面（core::api）驱动，不碰端口与内部模块；
  //          ③ 业务层内部不得成环（按强连通分量判定）。
  // 迁移期允许的现状违规进 tests/dependency-baseline.json；**条目一旦不再成立必须删除**（过期即失败）——
  // 这是"每拆一个分区就销一次账"的机器判据，不靠人看。
  const depBaselineFile = path.join(ROOT, "tests", "dependency-baseline.json");
  let depBaseline = null;
  try {
    depBaseline = JSON.parse(fs.readFileSync(depBaselineFile, "utf8"));
  } catch (e) {
    problems.push("依赖方向：基线不可读或不是合法 JSON：" + rel(depBaselineFile) + "（" + e.message + "）");
  }

  if (depBaseline) {
    // 模块级 `#[cfg(test)] mod tests { … }` 不是生产依赖边，剔除；
    // `#[cfg(test)]` 加在函数上的保留（它是这个文件里真实存在的一处代码）。
    const stripTestModules = (text) => {
      const lines = text.split(/\r?\n/);
      const out = [];
      for (let i = 0; i < lines.length; i++) {
        if (/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) {
          let j = i + 1;
          while (j < lines.length && lines[j].trim() === "") j++;
          if (j < lines.length && /^\s*(pub\s+)?mod\s+\w+/.test(lines[j])) {
            let k = j;
            while (k < lines.length && !/^\}/.test(lines[k])) k++;
            i = k;
            continue;
          }
        }
        out.push(lines[i]);
      }
      return out.join("\n");
    };
    // 引用归一到「层 + 模块」两级：crate::core::exec::diagnose_text → crate::core::exec。
    // 归一只为让基线稳定：同一依赖换个更细的写法不该让账本跳动。
    const normTarget = (full) => "crate::" + full.split("::").slice(0, 2).join("::");
    const LAYER_OF = (r) => {
      if (r.startsWith("src/core/")) return "core";
      if (r.startsWith("src/adapters/")) return "adapters";
      if (r.startsWith("src/presentation/")) return "presentation";
      if (r.startsWith("src/kernel/")) return "kernel";
      if (r.startsWith("src/tests/")) return "tests";
      if (r === "src/main.rs") return "main";
      return null;
    };
    // 层 → 它不得引用的层。core 不引用 adapters（端口由 core 定义、适配层实现，永不反向）。
    const FORBIDDEN = {
      // kernel 在最底层：无领域语义的机制，**不依赖任何人**。
      kernel: ["core", "adapters", "presentation"],
      core: ["adapters", "presentation"],
      adapters: ["presentation"],
    };

    const rsFiles = [];
    const collectRs = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) collectRs(p);
        else if (e.name.endsWith(".rs")) rsFiles.push(p);
      }
    };
    collectRs(path.join(ROOT, "src"));

    const reverse = new Set();
    const presentation = new Set();
    const coreGraph = {};
    for (const abs of rsFiles) {
      const f = rel(abs);
      const layer = LAYER_OF(f);
      if (layer === null) {
        // 新分区建目录时必须先归层，否则它会绕过门禁——**不留静默空洞**。
        problems.push("依赖方向：未分类的源码目录（门禁需要先把它归层）：" + f);
        continue;
      }
      if (layer === "tests" || layer === "main") continue;
      const text = stripTestModules(fs.readFileSync(abs, "utf8"));
      for (const m of text.matchAll(/crate::([a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)*)/g)) {
        const t = normTarget(m[1]);
        const targetLayer = t.split("::")[1];
        if ((FORBIDDEN[layer] || []).includes(targetLayer)) reverse.add(f + " -> " + t);
        if (layer === "presentation" && t !== "crate::core::api" && targetLayer !== "presentation") {
          presentation.add(f + " -> " + t);
        }
        if (layer === "core" && targetLayer === "core") {
          const self = path.basename(f).replace(/\.rs$/, "");
          const other = t.split("::")[2];
          if (other && other !== self) (coreGraph[self] = coreGraph[self] || new Set()).add(other);
        }
      }
    }

    // 强连通分量：> 1 个模块的分量 = 一个环。
    const coreSccs = [];
    {
      const idx = {}, low = {}, on = {}, stack = [];
      let counter = 0;
      const visit = (v) => {
        idx[v] = low[v] = counter++;
        stack.push(v);
        on[v] = true;
        for (const w of coreGraph[v] || []) {
          if (!(w in idx)) { visit(w); low[v] = Math.min(low[v], low[w]); }
          else if (on[w]) low[v] = Math.min(low[v], idx[w]);
        }
        if (low[v] === idx[v]) {
          const comp = [];
          let w;
          do { w = stack.pop(); on[w] = false; comp.push(w); } while (w !== v);
          if (comp.length > 1) coreSccs.push(comp.sort());
        }
      };
      for (const v of Object.keys(coreGraph)) if (!(v in idx)) visit(v);
    }
    const sortSccs = (list) => list.map((s) => s.slice().sort()).sort((a, b) => a.join(",").localeCompare(b.join(",")));

    // 每条规则比两次：**新增 = 失败；基线里已不成立 = 也失败**（强制销账）。
    const compare = (name, actualList, baselineList, hint) => {
      for (const a of actualList) {
        if (!baselineList.includes(a)) problems.push("依赖方向：" + hint + "：" + a);
      }
      for (const b of baselineList) {
        if (!actualList.includes(b)) {
          problems.push("依赖方向：基线豁免已过期，请从 " + rel(depBaselineFile) + " 的 " + name + " 删除：" + b);
        }
      }
    };
    compare("reverse", [...reverse].sort(), depBaseline.reverse || [], "业务/机制层不得反向依赖 adapters / presentation");
    compare("presentation", [...presentation].sort(), depBaseline.presentation || [], "presentation 只能经 crate::core::api 驱动");

    const actualCycles = sortSccs(coreSccs);
    const baselineCycles = sortSccs(depBaseline.coreCycles || []);
    const fmtCycles = (l) => (l.length ? l.map((s) => "[" + s.length + "] " + s.join(", ")).join("；") : "（无）");
    if (JSON.stringify(actualCycles) !== JSON.stringify(baselineCycles)) {
      problems.push(
        "依赖方向：业务层内部的环与基线不一致（分区拆出后请同步更新 " + rel(depBaselineFile) + " 的 coreCycles）：" +
        "\n    实际：" + fmtCycles(actualCycles) +
        "\n    基线：" + fmtCycles(baselineCycles)
      );
    }
  }

  return { problems, targets: targets.map((t) => t.name), testFiles: allTestFiles.length };
}

function main() {
  const steps = [];
/** 记一步：用时自动带上（步骤对象不关心时间时也不用写两遍）。 */
function pushStep(obj) {
  steps.push(Object.assign({ ms: Date.now() - stepStart }, obj));
}
  let doctor = null;
  announce("cargo build");
  console.log(FENCE_LIVE
    ? "[安全性] 已开启真机围栏测试：会在本机写权限（本工作区及其每一层祖先目录 + 命令用到的解释器安装目录）并创建容器 profile；一次性环境（CI / VM）里才建议开"
    : "[安全性] 安全模式：会改本机状态的测试已跳过（Windows 容器探针、端到端里的真实围栏写入）；要真跑加 --fence-live");
  const build = sh("cargo", ["build", "--color", "never"].concat(PROFILE === "release" ? ["--release"] : []));
  announceDone(build.code === 0 ? "完成" : "失败", build.code === 0 ? "" : build.log);
  pushStep({
    step: "cargo build",
    status: build.code === 0 ? "pass" : "fail",
    detail: build.code === 0 ? "" : (build.error || "") + " 日志：" + build.log,
    raw: build.code === 0 ? null : build.out.slice(-800),
  });
  if (build.code === 0 && fs.existsSync(BIN)) {
    const d = sh(BIN, ["--doctor"]);
    try { doctor = JSON.parse(d.out.trim()); } catch { doctor = { error: d.out.slice(0, 400) }; }
  }

  // ---- T0：硬失败（编译 + 结构审查） ----
  announce("T0 编译（--all-targets）");
  const check = sh("cargo", ["check", "--all-targets", "--color", "never"]);
  announceDone(check.code === 0 ? "完成" : "失败", check.code === 0 ? "" : check.log);
  pushStep({
    step: "T0 编译（--all-targets）",
    status: check.code === 0 ? "pass" : "fail",
    detail: check.code === 0 ? "" : (check.error || "") + " 日志：" + check.log,
    raw: check.code === 0 ? null : check.out.slice(-800),
  });

  announce("T0 结构审查");
  const audit = structuralAudit();
  announceDone(
    audit.problems.length ? "失败" : "完成",
    audit.problems.length ? audit.problems.length + " 条问题" : audit.targets.length + " 个目标 / " + audit.testFiles + " 个测试文件"
  );
  pushStep({
    step: "T0 结构审查",
    status: audit.problems.length ? "quality-fail" : "pass",
    detail: audit.problems.join("；"),
    raw: null,
  });

  // ---- T0：四项零容忍硬失败（基线机制已删除，见 docs/testing/quality-isolation.md） ----
  announce("T0 格式（fmt --check）");
  if (!toolAvailable("fmt")) {
    announceDone("env-skip", "cargo-fmt 未安装");
    pushStep({ step: "T0 格式（fmt --check）", status: "env-skip", detail: "cargo-fmt 未安装：rustup component add rustfmt" });
  } else {
    const r = sh("cargo", ["fmt", "--all", "--", "--check", "--color", "never"]);
    const files = [...fmtDeviations(r.out)];
    const ok = r.code === 0 && !files.length;
    announceDone(ok ? "完成" : "失败", ok ? "" : files.length + " 个文件有格式偏差");
    pushStep({
      step: "T0 格式（fmt --check）",
      status: ok ? "pass" : "quality-fail",
      detail: ok ? "" : "有格式偏差（跑 cargo fmt --all 后再提交）：" + files.join("、"),
      raw: ok ? null : r.out.slice(-1200),
    });
  }

  announce("T0 静态检查（clippy）");
  if (!toolAvailable("clippy")) {
    announceDone("env-skip", "cargo-clippy 未安装");
    pushStep({ step: "T0 静态检查（clippy）", status: "env-skip", detail: "cargo-clippy 未安装：rustup component add clippy" });
  } else {
    // --keep-going：-D warnings 会让首个失败的单元中断调度；加上它所有目标单元都编译完，计数才可复现。
    const r = sh("cargo", ["clippy", "--all-targets", "--all-features", "--keep-going", "--color", "never", "--", "-D", "warnings"]);
    const got = clippyLints(r.out);
    const total = Object.values(got).reduce((a, b) => a + b, 0);
    const ok = r.code === 0 && total === 0;
    announceDone(ok ? "完成" : "失败", ok ? "零告警" : total + " 处（" + Object.keys(got).join("、") + "）");
    pushStep({
      step: "T0 静态检查（clippy）",
      status: ok ? "pass" : "quality-fail",
      detail: ok ? "" : "clippy 有告警：" + Object.entries(got).map(([k, v]) => k + "=" + v).join("、") + "（设计取舍项要带理由 allow，见 docs/testing/quality-isolation.md）",
      raw: ok ? null : r.out.slice(-1200),
    });
  }

  announce("T0 编译告警");
  {
    const got = checkWarnings(check.out);
    const ok = got === 0;
    announceDone(ok ? "完成" : "失败", ok ? "零告警" : got + " 条");
    pushStep({
      step: "T0 编译告警",
      status: ok ? "pass" : "quality-fail",
      detail: ok ? "" : "rustc 告警 " + got + " 条（cargo check 的原文见日志）",
      raw: ok ? null : check.out.slice(-1200),
    });
  }

  announce("T0 依赖重复（cargo tree）");
  if (!toolAvailable("tree")) {
    announceDone("env-skip", "cargo-tree 不可用");
    pushStep({ step: "T0 依赖重复（cargo tree）", status: "env-skip", detail: "cargo tree 不可用" });
  } else {
    const r = sh("cargo", ["tree", "--duplicates", "--color", "never"]);
    const dups = duplicateCrates(r.out);
    const ok = r.code === 0 && !dups.length;
    announceDone(ok ? "完成" : "失败", ok ? "无重复" : dups.length + " 个重复 crate");
    pushStep({
      step: "T0 依赖重复（cargo tree）",
      status: ok ? "pass" : "quality-fail",
      detail: ok ? "" : "重复依赖（要有解释或治理记录）：" + dups.join("、"),
      raw: ok ? null : r.out.slice(-1200),
    });
  }

  // L1：crate 内联单元测试
  announce("L1 单元（--bin solomni）");
  const unit = sh("cargo", ["test", "--color", "never", "--bin", "solomni", "--", "--test-threads=1", "--nocapture"]);
  const uc = cargoCounts(unit.out);
  announceDone(unit.code === 0 ? "完成" : "失败", uc.passed + " passed / " + uc.failed + " failed");
  pushStep({
    step: "L1 单元（--bin solomni）",
    status: unit.code === 0 ? "pass" : "fail",
    detail: uc.passed + " passed / " + uc.failed + " failed",
    skips: skipsIn(unit.out),
    raw: unit.code === 0 ? null : unit.out.slice(-800),
  });

  // L2/L3：四个按平台分的测试目标逐一点名（缺目标即失败：新增测试文件必须挂到目标上）
  for (const t of PLATFORM_TARGETS) {
    announce("目标 " + t);
    const r = sh("cargo", ["test", "--color", "never", "--test", t, "--", "--test-threads=1", "--nocapture"]);
    const c = cargoCounts(r.out);
    const skips = skipsIn(r.out);
    const isOtherPlatform = t !== "cross-platform" && t !== OS_KEY;
    let status;
    if (r.code !== 0) status = "fail";
    else if (c.resultLines === 0) status = "fail";
    else if (c.passed === 0 && isOtherPlatform) status = "skip-platform";
    else status = "pass";
    announceDone(status === "fail" ? "失败" : status === "skip-platform" ? "本平台不适用" : "完成", c.passed + " passed / " + c.failed + " failed");
    pushStep({
      step: "目标 " + t,
      status: status,
      detail:
        c.passed + " passed / " + c.failed + " failed" +
        (status === "skip-platform" ? "（本平台不适用）" : "") +
        (skips.length ? "；env-skip " + skips.length + " 条" : ""),
      skips: skips,
      raw: status === "fail" ? r.out.slice(-800) : null,
    });
  }

  // 前端冒烟（自动发现同目录 *.smoke.cjs）
  announce("前端冒烟");
  const fe = sh(process.execPath, [path.join("src", "presentation", "web", "smoke.cjs")]);
  announceDone(fe.code === 0 ? "完成" : "失败", "");
  pushStep({
    step: "前端冒烟",
    status: fe.code === 0 && fe.out.includes("FRONTEND-SMOKE-OK") ? "pass" : "fail",
    detail: fe.out.trim().split(/\r?\n/).slice(-2).join(" / "),
    raw: fe.code === 0 ? null : fe.out.slice(-600),
  });

  // L4：端到端（有编排才跑；没有就是一条缺口，不装作跑过）
  const e2e = path.join(ROOT, "tests", "cross-platform", "e2e", "orchestrator.js");
  if (fs.existsSync(e2e)) {
    announce("L4 端到端");
    const r = sh(process.execPath, [e2e]);
    announceDone(r.code === 0 ? "完成" : "失败", "");
    pushStep({
      step: "L4 端到端",
      status: r.code === 0 && r.out.includes("E2E-OK") ? "pass" : "fail",
      detail: r.out.trim().split(/\r?\n/).slice(-2).join(" / "),
      raw: r.code === 0 ? null : r.out.slice(-800),
    });
  } else {
    pushStep({ step: "L4 端到端", status: "gap", detail: "cross-platform.e2e.not-in-runner（编排尚未迁入）" });
  }

  const gaps = gapLedgers();
  const globalGapsList = globalGaps();
  const failed = steps.filter((s) => s.status === "fail");
  const qualityFailed = steps.filter((s) => s.status === "quality-fail");
  const envSkips = steps.filter((s) => s.status === "env-skip");
  const skips = steps.flatMap((s) => s.skips || []);
  const report = {
    platform: process.platform,
    arch: process.arch,
    osKey: OS_KEY,
    profile: PROFILE,
    fenceLive: FENCE_LIVE,
    doctor: doctor,
    steps: steps.map((s) => ({ step: s.step, status: s.status, detail: s.detail, ms: s.ms })),
    envSkips: skips,
    quality: {
      failed: qualityFailed.length,
      steps: qualityFailed.map((s) => ({ step: s.step, detail: s.detail })),
    },
    globalGaps: globalGapsList,
    gaps: gaps,
    failed: failed.length,
  };
  fs.mkdirSync(path.dirname(REPORT), { recursive: true });
  fs.writeFileSync(REPORT, JSON.stringify(report, null, 2));

  console.log("");
  console.log("=== 测试汇总（" + process.platform + " " + process.arch + "，报告见 target/test-report.json）===");
  for (const s of steps) {
    console.log("  " + s.status.padEnd(13) + " " + s.step.padEnd(24) + " " + (s.detail || "") + (s.ms ? "  [" + (s.ms / 1000).toFixed(1) + "s]" : ""));
  }
  if (doctor && doctor.fence) console.log("  [doctor] 围栏 fs=" + doctor.fence.fs + " net=" + doctor.fence.net + " tree=" + doctor.fence.tree + "（" + doctor.fence.note + "）");
  for (const s of envSkips) console.log("  [env-skip] " + s.step + "：" + s.detail);
  for (const s of skips) console.log("  [env-skip] " + s);
  for (const s of qualityFailed) console.log("  [quality-fail] " + s.step + "：" + s.detail);
  for (const g of gaps) console.log("  [gap] " + g);
  for (const g of globalGapsList) console.log("  [global-gap] " + g);
  if (failed.length || qualityFailed.length) {
    for (const s of failed) {
      console.log("=== 失败详情：" + s.step + " ===");
      if (s.detail) console.log(s.detail);
      console.log(s.raw || "");
    }
    for (const s of qualityFailed) {
      console.log("=== 质量详情：" + s.step + " ===");
      if (s.detail) console.log(s.detail);
      console.log(s.raw || "");
    }
    console.log("TEST-REPORT-FAIL");
    process.exit(1);
  }
  console.log("TEST-REPORT-OK");
  if (gaps.length === 0) console.log("TEST-REPORT-ACCEPTED（本平台缺口账为空）");
  else console.log("[验收] 本平台仍有 " + gaps.length + " 条缺口未关账（tests/" + OS_KEY + "/gaps.yaml 与 tests/cross-platform/gaps.yaml）");
  if (globalGapsList.length) {
    console.log("[验收] 仍有 " + globalGapsList.length + " 条全局长期目标（tests/gaps.yaml）：" + globalGapsList.join("、"));
  }
}

main();
