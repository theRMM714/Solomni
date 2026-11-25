/**
 * 目的：解析呈现层的每屏入站面清单（\`docs/presentation/inbound.yaml\`）——一个受限子集，认不出就报错带行号。
 * 管：\`parseInboundText(text)\` → \`{ faces: Map<路径, 单元[]>, problems: [] }\`。
 * 不管：磁盘、源码扫描与判据（在 \`tests/gates/structure/presentation-inbound.js\`）。
 * 联动：门禁与 \`tests/lib/parsers.js\` 的自测都调它；改格式只改这一处。
 */

// 单元写法：单元（\`work\`）或单元/子单元（\`catalog/registry\`）；\`kernel\` 是机制层的入站面。
const UNIT_RE = /^(kernel|[a-z_][a-z0-9_]*(\/[a-z_][a-z0-9_]*)?)$/;
const PATH_RE = /^src\/presentation(\/[a-z_][a-z0-9_\/]*)\.rs$/;

/**
 * 目的：把清单正文读成「文件 → 允许取的单元」。
 * @param {string} text 文件正文
 * @returns {{faces: Map<string, string[]>, problems: string[]}}
 */
function parseInboundText(text) {
  const faces = new Map();
  const problems = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((raw, i) => {
    const line = raw.replace(/#.*$/, "").trim();
    if (line === "") return;
    const m = /^([^:\s]+):(.*)$/.exec(line);
    if (!m) {
      problems.push("呈现入站面：清单第 " + (i + 1) + " 行不是「路径: 单元…」：" + line);
      return;
    }
    const file = m[1];
    if (!PATH_RE.test(file)) {
      problems.push("呈现入站面：清单第 " + (i + 1) + " 行的路径必须形如 src/presentation/xxx.rs：" + file);
      return;
    }
    if (faces.has(file)) {
      problems.push("呈现入站面：清单里 " + file + " 出现了两次");
      return;
    }
    const units = [];
    for (const unit of m[2].trim().split(/\s+/).filter((u) => u !== "")) {
      if (!UNIT_RE.test(unit)) {
        problems.push("呈现入站面：清单第 " + (i + 1) + " 行的单元名不合规：" + unit);
        continue;
      }
      if (units.includes(unit)) {
        problems.push("呈现入站面：清单第 " + (i + 1) + " 行重复登记了单元：" + unit);
        continue;
      }
      units.push(unit);
    }
    faces.set(file, units.sort());
  });
  return { faces, problems };
}

module.exports = { parseInboundText };
