/**
 * 路径与匹配工具：仓库相对路径、只读文本、glob 匹配（--changed 用）。
 * 目的：给 run-tests.js 与 tests/gates 提供同一份路径口径（Windows 反斜杠一律归一为 /）。
 * 不引入第三方依赖：门禁要在没有 node_modules 的机器上直接跑。
 */
"use strict";
const fs = require("fs");
const path = require("path");

/** 仓库根相对路径（/ 分隔），门禁里的文件身份统一用这一种写法。 */
function rel(root, abs) {
  return path.relative(root, abs).replace(/\\/g, "/");
}

/** 只读文本（门禁不写被审查的文件）。 */
function read(file) {
  return fs.readFileSync(file, "utf8");
}

/**
 * 简化 glob：支持 **（跨目录）、*（单段内）、?（单字符），其余字符按字面量转义。
 * 目的：gate 用 inputs 声明"我读哪些路径"，--changed 从同一份声明反查，避免第二份影响面清单。
 */
function globToRegExp(pattern) {
  let out = "^";
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern[i];
    if (c === "*") {
      if (pattern[i + 1] === "*") {
        i++;
        if (pattern[i + 1] === "/") { i++; out += "(?:.*/)?"; }
        else out += ".*";
      } else out += "[^/]*";
      continue;
    }
    if (c === "?") { out += "[^/]"; continue; }
    out += c.replace(/[.+^$${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(out + "$");
}

function matchGlob(pattern, file) {
  return globToRegExp(pattern).test(String(file).replace(/\\/g, "/"));
}

/** 文件是否命中任一 glob。 */
function matchAny(patterns, file) {
  return (patterns || []).some((p) => matchGlob(p, file));
}

module.exports = { rel, read, globToRegExp, matchGlob, matchAny };
