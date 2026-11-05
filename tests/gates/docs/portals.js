/**
 * 文档审查·门户与链接：门户文档必须存在，各文件里的 Markdown 文档链接必须指向真实存在的文件。
 * 只查引用不查正文，避免把文档写法变成门禁；链接按所在文件的目录解析（根门户与 docs 细则都能抓到写错的同级引用）。
 */
"use strict";
const fs = require("fs");
const path = require("path");

function portals(root) {
  const problems = [];
  const rel = (f) => path.relative(root, f).replace(/\\/g, "/");
  const mdLinks = (text) => [...text.matchAll(/\]\(([^)#\s]+\.md)(?:#[^)]*)?\)/g)].map((m) => m[1]);
  const checkDocLinks = (absFile, label) => {
    const dir = path.dirname(absFile);
    for (const ref of mdLinks(fs.readFileSync(absFile, "utf8"))) {
      if (/^[a-z]+:/i.test(ref)) continue; // 外链不查
      if (!fs.existsSync(path.resolve(dir, ref))) problems.push(label + " 引用的文档不存在：" + ref);
    }
  };
  const portalFiles = ["README.md", "README_EN.md", "AGENTS.md", "ARCHITECTURE.md", "PRODUCT.md", "MODULE_SPEC.md", "SYSTOOL.md", "RUNTIME_SPEC.md", "REGISTRY_SPEC.md", "TESTING.md"];
  for (const p of portalFiles) {
    const abs = path.join(root, p);
    if (!fs.existsSync(abs)) { problems.push("缺门户文档：" + p); continue; }
    checkDocLinks(abs, p);
  }
  const docsDir = path.join(root, "docs");
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
  return problems;
}

module.exports = { portals };
