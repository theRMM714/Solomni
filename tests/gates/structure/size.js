/**
 * 结构审查·文件粒度门禁：一个文件只装一件事，超过上限就是职责没拆完。
 * 判据按文件角色分档（实现 / 测试用例 / 测试支撑），实现按"生产行"算（内联测试块不计）；
 * 上限是**硬线**，没有棘轮、没有历史快照——唯一的例外是 EXEMPT 里逐条写明理由的账，条目过期即失败。
 * 内联测试只许留在实现文件里做纯函数与全内存的断言，且单块不超过内联上限。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");

/** 各档上限（行）：hard 必须遵守，soft 只回执不动成败。 */
const LIMITS = {
  impl: { prodSoft: 800, prodHard: 1200, totalHard: 1500 },
  test: { soft: 1500, hard: 2000 },
  support: { soft: 800, hard: 1200 },
  inline: { soft: 200, hard: 400 },
};

/** 逐条豁免：只给"一个不变量、拆开就要跨文件共享同一套状态"的文件；不再超线时必须删掉条目。 */
const EXEMPT = {
  "src/kernel/detail/confine/windows/record.rs":
    "Windows 围栏的授权台账：一条授权的建立、写前快照、写后核对、按条撤销与启动对账共用同一份盘上格式与内存状态，拆开就要把这些规则复制成多份",
};

const HEADER_BUDGET = 120;

/** 把字符串、字符与注释抹成空白（保留换行），供括号配平使用。 */
function mask(text) {
  const out = text.split("");
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === '"' || c === "'") {
      const q = c;
      out[i] = " ";
      i++;
      while (i < text.length) {
        const d = text[i];
        if (d === "\\") {
          out[i] = " ";
          if (i + 1 < text.length && text[i + 1] !== "\n") out[i + 1] = " ";
          i += 2;
          continue;
        }
        out[i] = d === "\n" ? "\n" : " ";
        if (d === q) {
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") out[i++] = " ";
      continue;
    }
    if (c === "/" && text[i + 1] === "*") {
      let depth = 1;
      out[i] = out[i + 1] = " ";
      i += 2;
      while (i < text.length && depth > 0) {
        if (text[i] === "/" && text[i + 1] === "*") {
          depth++;
          out[i] = out[i + 1] = " ";
          i += 2;
          continue;
        }
        if (text[i] === "*" && text[i + 1] === "/") {
          depth--;
          out[i] = out[i + 1] = " ";
          i += 2;
          continue;
        }
        out[i] = text[i] === "\n" ? "\n" : " ";
        i++;
      }
      continue;
    }
    i++;
  }
  return out.join("");
}

/** 文件里 \`#[cfg(test)] mod x { ... }\` 块的行区间（1-based，闭区间），以及它指向的兄弟测试文件。 */
function testModules(text) {
  const lines = text.split(/\r?\n/);
  const masked = mask(text).split("\n");
  const blocks = [];
  const siblings = [];
  for (let i = 0; i < lines.length; i++) {
    const attr = lines[i].match(/^\s*#\[cfg\(test\)\]\s*(.*)$/);
    if (!attr) continue;
    let j = i;
    let text = attr[1];
    if (!text) {
      j = i + 1;
      while (j < lines.length && lines[j].trim() === "") j++;
      if (j >= lines.length) continue;
      text = lines[j];
    }
    const decl = text.match(/^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*(;|\{)/);
    if (!decl) continue;
    if (decl[2] === ";") {
      siblings.push(decl[1]);
      continue;
    }
    if (j < i) j = i;
    let depth = 0;
    let end = j;
    for (let k = j; k < lines.length; k++) {
      let closed = false;
      for (const ch of masked[k]) {
        if (ch === "{") depth++;
        else if (ch === "}") {
          depth--;
          if (depth === 0) {
            closed = true;
            break;
          }
        }
      }
      if (closed) {
        end = k;
        break;
      }
    }
    blocks.push([i + 1, end + 1]);
    i = end;
  }
  return { blocks, siblings };
}

/** 声明了测试专属子模块的文件集合（相对路径），它们是测试代码而不是生产代码。 */
function testFileSet(root, files) {
  const out = new Set();
  for (const f of files) {
    const text = fs.readFileSync(path.join(root, f), "utf8");
    const { siblings } = testModules(text);
    // Rust 的子模块目录：`a/b.rs` 的子模块在 `a/b/` 下，`a/b/mod.rs` 的在 `a/b/` 下。
    const stem = f.replace(/\.rs$/, "");
    const dir = path.posix.basename(stem) === "mod" ? path.posix.dirname(stem) : stem;
    for (const name of siblings) {
      for (const cand of [dir + "/" + name + ".rs", dir + "/" + name + "/mod.rs"]) {
        if (fs.existsSync(path.join(root, cand))) out.add(cand);
      }
    }
  }
  return out;
}

function walk(root, dir) {
  const files = [];
  const rec = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) rec(p);
      else if (e.name.endsWith(".rs")) files.push(rel(root, p));
    }
  };
  rec(path.join(root, dir));
  return files;
}

/** 文件角色：实现 / 测试用例 / 测试支撑。 */
function roleOf(f, testFiles) {
  if (f.startsWith("src/tests/doubles/") || f.startsWith("src/tests/builders/")) return "support";
  if (f.startsWith("src/tests/") || testFiles.has(f)) return "test";
  return "impl";
}

/** 文件头的 //! 段落：超过预算说明"这个文件一句话说不清自己是什么"。 */
function headerSlot(raw) {
  let body = "";
  for (const line of raw.split(/\r?\n/).slice(0, 40)) {
    if (!line.startsWith("//!")) {
      if (body) break;
      continue;
    }
    body += line.replace(/^\/\/!\s?/, "");
  }
  if (!body) return "缺";
  return body.replace(/\s+/g, "").length > HEADER_BUDGET ? "写不下" : "能";
}

/** 扫描：每个文件的行数事实与内联测试块。 */
function scan(root) {
  const files = walk(root, "src");
  const testFiles = testFileSet(root, files);
  const rows = [];
  for (const f of files) {
    const raw = fs.readFileSync(path.join(root, f), "utf8");
    const lines = raw.split(/\r?\n/);
    if (lines[lines.length - 1] === "") lines.pop();
    const { blocks } = testModules(raw);
    let inline = 0;
    for (const [a, b] of blocks) inline += b - a + 1;
    const role = roleOf(f, testFiles);
    rows.push({
      file: f,
      role,
      total: lines.length,
      inline,
      prod: lines.length - inline,
      header: headerSlot(raw),
    });
  }
  return rows;
}

function limitsFor(role) {
  return role === "test" ? LIMITS.test : role === "support" ? LIMITS.support : LIMITS.impl;
}

/** 超过硬线时的文案：说清"哪一档、超多少、该往哪拆"。 */
function hardProblem(row) {
  const lim = limitsFor(row.role);
  if (row.role === "impl") {
    if (row.prod > lim.prodHard) {
      return "文件粒度：实现文件生产行超过 " + lim.prodHard + "（" + row.file + "，" + row.prod + " 行）：按职责拆，不要为过线机械切";
    }
    if (row.total > lim.totalHard) {
      return "文件粒度：实现文件总行超过 " + lim.totalHard + "（" + row.file + "，" + row.total + " 行）：测试搬去 tests.rs，实现按职责拆";
    }
    return null;
  }
  if (row.total > lim.hard) {
    const what = row.role === "test" ? "测试用例文件" : "测试支撑文件";
    const hint = row.role === "test" ? "按用例族拆子模块，用例名保持不变" : "按端口族拆，保持替身契约不变";
    return "文件粒度：" + what + "超过 " + lim.hard + "（" + row.file + "，" + row.total + " 行）：" + hint;
  }
  return null;
}

function fileSize(root) {
  const problems = [];
  const notes = [];
  const rows = scan(root);
  for (const row of rows) {
    const lim = limitsFor(row.role);
    const exempt = EXEMPT[row.file];
    const over = hardProblem(row);
    if (over && !exempt) problems.push(over);
    if (over && exempt) notes.push("[hold] " + row.file + "  " + row.prod + " 行  豁免：" + exempt);
    // 回执：软线以上列出事实（行数、IO 次数、impl 块数、目的槽），不动成败。
    const soft = row.role === "impl" ? row.prod > lim.prodSoft : row.total > lim.soft;
    if (soft) {
      const prod = row.role === "impl";
      const size = prod ? row.prod : row.total;
      const io = prod ? (fs.readFileSync(path.join(root, row.file), "utf8").match(/std::fs|std::process::Command|ureq|reqwest|std::net/g) || []).length : 0;
      notes.push("[big] " + row.file + "  " + size + " 行  io=" + io + "  目的槽=" + row.header);
    }
    // 内联测试：只许纯函数与全内存断言，且不能把实现文件撑成第二个测试文件。
    if (row.role === "impl" && row.inline > LIMITS.inline.hard) {
      problems.push(
        "文件粒度：内联测试块超过 " + LIMITS.inline.hard + " 行（" + row.file + "，" + row.inline + " 行）：改成 #[cfg(test)] mod tests; 指向同目录 tests.rs"
      );
    } else if (row.role === "impl" && row.inline > LIMITS.inline.soft) {
      notes.push("[inline] " + row.file + "  内联测试 " + row.inline + " 行");
    }
  }
  // 豁免过期：条目还在、文件却不再超线，说明这条账该销。
  for (const f of Object.keys(EXEMPT)) {
    const row = rows.find((r) => r.file === f);
    if (!row) {
      problems.push("文件粒度：豁免条目指向的文件不存在，请从 tests/gates/structure/size.js 的 EXEMPT 删除：" + f);
      continue;
    }
    if (!hardProblem(row)) problems.push("文件粒度：豁免已过期（该文件已不超线），请从 EXEMPT 删除：" + f);
  }
  return { problems, notes };
}

module.exports = { fileSize, scan, LIMITS, EXEMPT };
