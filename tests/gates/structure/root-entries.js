/**
 * 根目录门禁：产品根只放"哪一类东西"，不放"某一件事的产物"。
 * 判据一：一级条目必须在白名单内（源码 / 文档 / 随包只读内容 / 开发环境）。新增条目要么进 `data/`，
 *   要么在本文件登记并写清归谁——根目录再变乱时立刻红，而不是等 code review 发现。
 * 判据二：旧的介质名（`.home` / `.vm` / `logs` / `session` / `runtimes`）不许回到根目录：产品只写 `data/`。
 * 判据三：模块源码树（`modules/<id>`）里不许长 `userdata/`——私有区在 `data/modules/<id>/userdata`
 *   （`content` 单元确保存在）；构建产物 `build/` 在模块目录里是正常的（`builds` 的 cwd 就在模块根），
 *   由 `.gitignore` 忽略，不进版本库、也不进数据根。
 * 零豁免：命中即失败，没有基线文件可销账。
 */
"use strict";
const fs = require("fs");
const path = require("path");

/** 入库的目录：源码、文档、随包只读内容、产品数据根。 */
const TRACKED_DIRS = [
  "src",
  "tests",
  "docs",
  "modules",
  "dependencies",
  "demo",
  "locales",
  "prompts",
  "systools",
  "bin",
  "data",
  ".github",
  ".cargo",
  ".vscode",
];
/** 入库的文件：构建清单、工具链入口、门户与规范。\`*.md\` 单独按扩展名放行（根文档由 root-router 判）。 */
const TRACKED_FILES = [
  "Cargo.toml",
  "Cargo.lock",
  "build.rs",
  "deny.toml",
  "package.json",
  "package-lock.json",
  "tsconfig.json",
  "layers.yaml",
  "links.yaml",
  "dependencies.yaml",
  "systool_gaps.yaml",
  "dev.js",
  "dev-tools.js",
  "env.js",
  "pins.js",
  "run-tests.js",
  "run-hygiene.js",
  "setup-dev.js",
  "start.js",
  "start.bat",
  "start.sh",
  "test.bat",
  "test.sh",
  ".gitattributes",
  ".gitignore",
  "LICENSE",
];
/** 本机产物与开发环境：都在 `.gitignore` 里，允许出现在根目录，但不入库。 */
const LOCAL_ONLY = [".git", ".tmp", ".tools", ".local", ".shots", ".worktrees", "platform", "target", "node_modules", "Structure.html"];
const LOCAL_PATTERNS = [/^mutants\.out/, /^%SystemDrive%$/];
/** 旧介质名：产品盘面收进 `data/` 之后，这些名字再回到根目录就说明有人在绕过 `layout`。 */
const OLD_MEDIA = [".home", ".vm", "logs", "session", "runtimes"];

/** 目的：判一个一级条目是否被登记过（目录 / 文件 / 本机产物 / 根文档四类之一）。 */
function allowed(name, isDir) {
  if (isDir && TRACKED_DIRS.includes(name)) return true;
  if (!isDir && (TRACKED_FILES.includes(name) || name.endsWith(".md"))) return true;
  if (LOCAL_ONLY.includes(name)) return true;
  return LOCAL_PATTERNS.some((re) => re.test(name));
}

/** 目的：扫产品根的一级条目与模块源码树，返回问题清单。 */
function rootEntries(root) {
  const problems = [];
  const entries = fs.readdirSync(root, { withFileTypes: true });
  for (const e of entries) {
    if (OLD_MEDIA.includes(e.name)) {
      problems.push(
        "根目录：出现旧介质「" + e.name + "」——产品只写 data/，路径经 kernel::api::layout 取",
      );
      continue;
    }
    if (!allowed(e.name, e.isDirectory())) {
      problems.push(
        "根目录：出现未登记的条目「" +
          e.name +
          "」——是产品介质就进 data/；是开发环境或产物就登记进 tests/gates/structure/root-entries.js 并写清归谁",
      );
    }
  }
  const modules = path.join(root, "modules");
  if (fs.existsSync(modules)) {
    for (const e of fs.readdirSync(modules, { withFileTypes: true })) {
      if (!e.isDirectory()) continue;
      const userdata = path.join(modules, e.name, "userdata");
      if (fs.existsSync(userdata)) {
        problems.push(
          "根目录：模块源码树里长了 modules/" +
            e.name +
            "/userdata——私有区在 data/modules/" +
            e.name +
            "/userdata（content 单元确保存在）",
        );
      }
    }
  }
  return {
    problems,
    notes: ["根目录：判了 " + entries.length + " 个一级条目（白名单读自本文件）"],
  };
}

module.exports = { rootEntries };
