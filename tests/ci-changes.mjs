#!/usr/bin/env node
/**
 * CI 变更分流：把一次 PR 的改动分成两路。
 * - code_changed：至少一个非叙述型文档路径 → 三平台（quality / e2e）+ 结构审查；
 * - docs_changed：至少一个文档路径 → 文档审查 + 结构审查。
 * 结构审查两路都要跑：代码变更会破坏代码↔文档一致性，文档变更同样会。
 * 机器契约文档（docs/presentation/contracts.md）算代码：它的 ROUTES 表被 Rust 测试消费；
 * 但它同时是文档，所以文档审查也跑（两个谓词不是互斥的）。
 * 失败兜底：取不到 diff、未知路径、空 diff 一律两路都跑（宁可全量，不可漏测）。
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
// 机器契约型文档：被代码/测试直接消费，改它们必须跑三平台（当前由 src/tests/routes.rs 的 include_str! 消费）。
const CONTRACT_DOCS = new Set(["docs/presentation/contracts.md"]);
const EXACT_DOCS = new Set(["LICENSE"]);

/** 把 Git 的 / 分隔路径归一（防御性：不依赖调用方）；分类只认仓库根相对路径。 */
function normPath(file) {
  return String(file).replace(/\\/g, "/").replace(/^\.\//, "");
}

/** 叙述型文档：.md、docs/**、LICENSE。机器契约文档显式排除（它必须进代码门禁）。 */
function isDocPath(file) {
  const p = normPath(file);
  if (CONTRACT_DOCS.has(p)) return false;
  if (EXACT_DOCS.has(p)) return true;
  if (p.startsWith("docs/")) return true;
  if (p.endsWith(".md")) return true;
  return false;
}

/** 文档审查的输入：文档文件本身（含机器契约文档——它也是文档，链接仍要查）。 */
function isDocsReviewPath(file) {
  const p = normPath(file);
  if (EXACT_DOCS.has(p)) return true;
  if (p.startsWith("docs/")) return true;
  if (p.endsWith(".md")) return true;
  return false;
}

/**
 * 纯判定：
 * - code：存在非叙述型文档路径（含 contracts.md）→ 三平台 + 结构审查；
 * - docs：存在文档路径（含 contracts.md）→ 文档审查 + 结构审查。
 * 空 diff 不在这里兜底（两个都 false），由 main 按全量处理。
 */
export function classify(files) {
  if (!files || !files.length) return { code: false, docs: false };
  return {
    code: files.some((f) => !isDocPath(f)),
    docs: files.some(isDocsReviewPath),
  };
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
    { files: ["README.md"], code: false, docs: true },
    { files: ["docs/a/b.md"], code: false, docs: true },
    { files: ["docs/img/x.png"], code: false, docs: true },
    { files: ["docs\\a.md"], code: false, docs: true },
    { files: ["LICENSE"], code: false, docs: true },
    { files: [".github/pull_request_template.md"], code: false, docs: true },
    { files: ["modules/x/README.md"], code: false, docs: true },
    { files: ["dependencies/THIRD-PARTY-NOTICES.md"], code: false, docs: true },
    // 机器契约文档：代码门禁与文档审查都要跑。
    { files: ["docs/presentation/contracts.md"], code: true, docs: true },
    { files: ["README.md", "src/main.rs"], code: true, docs: true },
    { files: ["README.md", "docs/presentation/contracts.md"], code: true, docs: true },
    { files: ["Cargo.toml"], code: true, docs: false },
    { files: ["Cargo.lock"], code: true, docs: false },
    { files: ["prompts/shared/texts.yaml"], code: true, docs: false },
    { files: ["systools/tools.yaml"], code: true, docs: false },
    { files: ["links.yaml"], code: true, docs: false },
    { files: ["dependencies.yaml"], code: true, docs: false },
    { files: ["tests/gaps.yaml"], code: true, docs: false },
    { files: [".github/workflows/test.yml"], code: true, docs: false },
    { files: ["demo/x.mjs"], code: true, docs: false },
    { files: [], code: false, docs: false },
  ];
  for (const c of cases) {
    const got = classify(c.files);
    assert.deepEqual(got, { code: c.code, docs: c.docs }, "分类样例不成立：" + JSON.stringify(c.files));
  }
  const consumed = docsConsumedByCode();
  assert.ok(consumed.includes("docs/presentation/contracts.md"), "自检应发现 routes.rs 消费的 docs/presentation/contracts.md");
  for (const p of consumed) {
    assert.equal(classify([p]).code, true, "被代码 include 的文档必须走代码门禁：" + p);
    assert.equal(classify([p]).docs, true, "被代码 include 的文档也在 docs/ 下，文档审查也要跑：" + p);
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

function writeOutputs(codeChanged, docsChanged) {
  const out = process.env.GITHUB_OUTPUT;
  if (!out) return;
  fs.appendFileSync(out, "code_changed=" + (codeChanged ? "true" : "false") + "\n");
  fs.appendFileSync(out, "docs_changed=" + (docsChanged ? "true" : "false") + "\n");
}

function writeSummary(codeChanged, docsChanged, files) {
  const summary = process.env.GITHUB_STEP_SUMMARY;
  if (!summary) return;
  const shown = files.slice(0, 200);
  const body = "### 变更分流\n\n- code_changed：" + codeChanged + "\n- docs_changed：" + docsChanged + "\n- 文件数：" + files.length + "\n\n"
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
  // 兜底是全量：没有 base/head、空 diff、取不到 diff 都让两路都跑。
  let codeChanged = true;
  let docsChanged = true;
  if (base && head) {
    try {
      files = changedFiles(base, head);
      const c = classify(files);
      if (files.length) {
        codeChanged = c.code;
        docsChanged = c.docs;
      } else {
        console.log("::warning title=变更分流::空 diff，按全量 CI 处理");
      }
    } catch (e) {
      console.log("::warning title=变更分流::取不到 PR diff，按全量 CI 处理：" + String((e && e.message) || e).slice(0, 300));
    }
  } else {
    console.log("[变更分流] 没有 PR base/head（手动派发？），按全量 CI 处理");
  }

  writeOutputs(codeChanged, docsChanged);
  writeSummary(codeChanged, docsChanged, files);
  console.log("[变更分流] 文件 " + files.length + " 个；code_changed=" + codeChanged + "；docs_changed=" + docsChanged);
  if (codeChanged) console.log("[变更分流] 跑三平台（quality / e2e）+ 结构审查");
  if (docsChanged) console.log("[变更分流] 跑文档审查 + 结构审查");
  if (!codeChanged && !docsChanged) console.log("[变更分流] 无可用变更，按全量兜底");
}

const invoked = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) main();
