/**
 * 结构审查·跨平台文本门禁：平台专属代码本地不编译（unix 的 #[cfg] 在 Windows 上不存在，反之亦然），
 * 这些错只在别的平台炸，所以只能按文本挡住：FenceSpec 字面量必带 ro、macOS/Linux 探针标记不许串、
 * 批处理脚本只允许 ASCII、#[ignore] 一律禁止。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { normPath, batchNonAscii } = require("../../lib/parsers.js");

/** FenceSpec 字面量完整性：每个字面量都要写全字段（ro 与 rw 同层）；只认字面量，排除 struct/impl/fn。 */
function fenceLiterals(root) {
  const problems = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) { walk(p); continue; }
      if (!e.name.endsWith(".rs")) continue;
      const lines = fs.readFileSync(p, "utf8").split(/\r?\n/);
      lines.forEach((line, i) => {
        const t = line.trim();
        if (!t.endsWith("FenceSpec {")) return;
        if (t.includes("->")) return;
        if (/^(pub\s+)?(struct|enum|impl|fn)\b/.test(t)) return;
        let end = i + 1;
        while (end < lines.length && !/^\s*\};?\s*$/.test(lines[end])) end++;
        const body = lines.slice(i + 1, end).join("\n");
        if (!/\bro:/.test(body)) {
          problems.push(rel(root, p) + ":" + (i + 1) + " 的 FenceSpec 字面量缺 ro（平台专属代码本地不编译，CI 才会炸）");
        }
      });
    }
  };
  walk(path.join(root, "src"));
  return problems;
}

/** 批处理脚本只允许 ASCII：cmd 按代码页解释字节，非 ASCII 注释会吞掉下一行（只在中文 Windows 上炸）。 */
function batchAscii(root) {
  const problems = [];
  // 只扫仓库内容：工具链、产物、运行期记录与端到端夹具都不是仓库的一部分。
  const SKIP = new Set([
    ".git", "node_modules", "platform", "target", ".tools", ".vm",
    ".home", "session", "logs", "runtimes", "mutants.out", "mutants.out.old",
  ]);
  const walkDir = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      const r = normPath(path.relative(root, p));
      if (e.isDirectory()) {
        if (SKIP.has(e.name) || r === "tests/cross-platform/e2e/root" || r.startsWith("tests/cross-platform/e2e/root/")) continue;
        walkDir(p);
        continue;
      }
      if (!/\.(cmd|bat)$/i.test(e.name)) continue;
      const hit = batchNonAscii(fs.readFileSync(p));
      if (hit.bom) {
        problems.push(r + " 以 UTF-8 BOM 开头（cmd 会把首行读成不存在的命令）：批处理脚本只允许 ASCII，见 MODULE_SPEC 的「跨平台」");
      }
      if (hit.line) {
        problems.push(r + ":" + hit.line + " 含非 ASCII 字节（cmd 按代码页解释会吞掉下一行）：批处理脚本只允许 ASCII，见 MODULE_SPEC 的「跨平台」");
      }
    }
  };
  walkDir(root);
  return problems;
}

/** 平台探针标记不许串：常量名写串了只在那个平台上炸（本地不编译，按文本挡住这一对）。 */
function crossMarks(root) {
  const problems = [];
  const crossPlatformMarks = [
    ["tests/macos", "RULES_REJECTED_MARK", "Linux 的标记出现在 macOS 探针里"],
    ["tests/linux", "PROFILE_REJECTED_MARK", "macOS 的标记出现在 Linux 探针里"],
  ];
  for (const [dir, needle, why] of crossPlatformMarks) {
    const abs = path.join(root, dir);
    if (!fs.existsSync(abs)) continue;
    const walkMarks = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) { walkMarks(p); continue; }
        if (!e.name.endsWith(".rs")) continue;
        fs.readFileSync(p, "utf8").split(/\r?\n/).forEach((line, i) => {
          if (line.includes(needle) && !line.trim().startsWith("//")) {
            problems.push(rel(root, p) + ":" + (i + 1) + " " + why + "（本地不编译，CI 才会炸）");
          }
        });
      }
    };
    walkMarks(abs);
  }
  return problems;
}

/** #[ignore] 一律不许：跳过必须显式、可判（进缺口账或 env-skip），不能用测试框架的 ignore 藏起来。 */
function ignoreBan(root) {
  const problems = [];
  for (const d of [path.join(root, "src"), path.join(root, "tests")]) {
    const scanIgnore = (dir) => {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) { scanIgnore(p); continue; }
        if (!e.name.endsWith(".rs")) continue;
        fs.readFileSync(p, "utf8").split(/\r?\n/).forEach((line, i) => {
          if (/^\s*#\[ignore/.test(line)) problems.push("#[ignore] 被禁止（跳过要如实记账，不能用框架 ignore）：" + rel(root, p) + ":" + (i + 1));
        });
      }
    };
    scanIgnore(d);
  }
  return problems;
}

module.exports = { fenceLiterals, batchAscii, crossMarks, ignoreBan };
