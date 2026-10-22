#!/usr/bin/env node
/**
 * CI 变更分流：判断一次 PR 是否只改了叙述型文档。
 * 判据：doc-only 的 PR 跳过三平台重 job，只跑文档门禁；机器契约文档仍走代码门禁
 * （被 include_str!/include_bytes! 消费的 docs/** 由自检强制归代码门禁）。
 * 失败兜底：取不到 diff、未知路径一律 docs_only=false（宁可全量，不可漏测）。
 * 约束：只依赖 Node 内置模块——CI 会从 base 把它复制到 target/ 再执行，相对导入会失效。
 */
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// 分类器在 CI 里会从 base 复制到 target/ 再执行（防止 PR 改自己的分类逻辑）；
// 那时用 SOLOMNI_ROOT 指回仓库根，其余调用方式按脚本所在目录推断。
const ROOT = process.env.SOLOMNI_ROOT
  ? path.resolve(process.env.SOLOMNI_ROOT)
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// 机器契约型文档：被代码/测试直接消费，改它们必须跑代码门禁（当前由 src/tests/routes.rs 的 include_str! 消费）。
const CONTRACT_DOCS = new Set(["docs/presentation/contracts.md"]);
const EXACT_DOCS = new Set(["LICENSE"]);

/** 把 Git 的 / 分隔路径归一（防御性：不依赖调用方）；分类只认仓库根相对路径。 */
function normPath(file) {
  return String(file).replace(/\\/g, "/").replace(/^\.\//, "");
}

/** 是否是叙述型文档：.md、docs/**、LICENSE。机器契约文档显式排除。 */
function isDocPath(file) {
  const p = normPath(file);
  if (CONTRACT_DOCS.has(p)) return false;
  if (EXACT_DOCS.has(p)) return true;
  if (p.startsWith("docs/")) return true;
  if (p.endsWith(".md")) return true;
  return false;
}

/**
 * 纯判定：只有全部文件都是叙述型文档才算 docs-only。
 * 空 diff 不算 docs-only：没有可验收的变更，交回全量入口处理。
 */
export function classify(files) {
  if (!files || !files.length) return false;
  return files.every(isDocPath);
}

/** 被 Rust 代码 include 的 docs/**：这些文档改了可能破坏编译或契约测试，必须走代码门禁。 */
function docsConsumedByCode() {
  const found = new Set();
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { walk(p); continue; }
      if (!e.name.endsWith(".rs")) continue;
      const text = fs.readFileSync(p, "utf8");
      // 只认直接字符串字面量；concat! 等拼装形式不在自检范围（新增消费点应改成直接字面量）。
      for (const m of text.matchAll(/include_(?:str|bytes)!\s*\(\s*"([^"]+)"\s*\)/g)) {
        const abs = path.resolve(path.dirname(p), m[1]);
        const rel = path.relative(ROOT, abs).split(path.sep).join("/");
        if (rel.startsWith("docs/") && fs.existsSync(abs)) found.add(rel);
      }
    }
  };
  walk(path.join(ROOT, "src"));
  walk(path.join(ROOT, "tests"));
  return [...found].sort();
}

function selfTest() {
  const cases = [
    { files: ["README.md"], want: true },
    { files: ["docs/a/b.md"], want: true },
    { files: ["docs/img/x.png"], want: true },
    { files: ["docs\\a.md"], want: true },
    { files: ["LICENSE"], want: true },
    { files: [".github/pull_request_template.md"], want: true },
    { files: ["modules/x/README.md"], want: true },
    { files: ["dependencies/THIRD-PARTY-NOTICES.md"], want: true },
    { files: ["docs/presentation/contracts.md"], want: false },
    { files: ["README.md", "src/main.rs"], want: false },
    { files: ["README.md", "docs/presentation/contracts.md"], want: false },
    { files: ["Cargo.toml"], want: false },
    { files: ["Cargo.lock"], want: false },
    { files: ["prompts/shared/texts.yaml"], want: false },
    { files: ["systools/tools.yaml"], want: false },
    { files: ["links.yaml"], want: false },
    { files: ["dependencies.yaml"], want: false },
    { files: ["tests/gaps.yaml"], want: false },
    { files: [".github/workflows/test.yml"], want: false },
    { files: ["demo/x.mjs"], want: false },
    { files: [], want: false },
  ];
  for (const c of cases) {
    assert.equal(classify(c.files), c.want, "分类样例不成立：" + JSON.stringify(c.files));
  }
  const consumed = docsConsumedByCode();
  assert.ok(consumed.includes("docs/presentation/contracts.md"), "自检应发现 routes.rs 消费的 docs/presentation/contracts.md");
  for (const p of consumed) {
    assert.equal(classify([p]), false, "被代码 include 的文档必须走代码门禁：" + p);
  }
  console.log("[变更分流] 自检通过：分类样例 " + cases.length + " 条，include 文档 " + consumed.length + " 个");
}

function git(args, opts = {}) {
  return execFileSync("git", args, Object.assign({ cwd: ROOT, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 }, opts));
}

function ensureCommit(sha) {
  try {
    git(["cat-file", "-e", sha + "^{commit}"], { stdio: "ignore" });
    return;
  } catch {
    // PR 的 base 可能晚于 head 分叉点，本地没有就打向 base 仓取；取不到由调用方兜底成全量。
  }
  git(["fetch", "--no-tags", "--depth=1", "origin", sha], { stdio: ["ignore", "ignore", "pipe"] });
}

/** PR 的文件清单：三点 diff + 关闭重命名检测，改名会同时给出旧/新路径，偏保守。 */
function changedFiles(base, head) {
  ensureCommit(base);
  ensureCommit(head);
  const out = git(["diff", "--name-only", "-z", "--no-renames", base + "..." + head]);
  return out.split("\0").filter(Boolean);
}

function writeOutputs(docsOnly) {
  const out = process.env.GITHUB_OUTPUT;
  if (out) fs.appendFileSync(out, "docs_only=" + (docsOnly ? "true" : "false") + "\n");
}

function writeSummary(docsOnly, files) {
  const summary = process.env.GITHUB_STEP_SUMMARY;
  if (!summary) return;
  const shown = files.slice(0, 200);
  const body = "### 变更分流\n\n- docs_only：" + docsOnly + "\n- 文件数：" + files.length + "\n\n"
    + (files.length ? "    " + shown.join("\n    ") + (files.length > shown.length ? "\n    …（其余省略）" : "") + "\n" : "（无文件变更）\n");
  fs.appendFileSync(summary, body);
}

function main() {
  const selfOnly = process.argv.includes("--self-test");
  try {
    selfTest();
  } catch (e) {
    console.error("[变更分流] 自检失败：" + (e && e.message ? e.message : e));
    process.exit(1);
  }
  if (selfOnly) return;

  const base = (process.env.BASE_SHA || "").trim();
  const head = (process.env.HEAD_SHA || "").trim();
  let files = [];
  let docsOnly = false;
  if (base && head) {
    try {
      files = changedFiles(base, head);
      docsOnly = classify(files);
    } catch (e) {
      console.log("::warning title=变更分流::取不到 PR diff，按全量 CI 处理：" + String((e && e.message) || e).slice(0, 300));
      docsOnly = false;
    }
  } else {
    console.log("[变更分流] 没有 PR base/head（手动派发？），按全量 CI 处理");
  }

  writeOutputs(docsOnly);
  writeSummary(docsOnly, files);
  console.log("[变更分流] 文件 " + files.length + " 个；docs_only=" + docsOnly);
  if (!docsOnly) console.log("[变更分流] 走代码门禁（三平台 quality / e2e）");
  else console.log("[变更分流] 走文档门禁（只跑 run-tests.js --docs）");
}

const invoked = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) main();
