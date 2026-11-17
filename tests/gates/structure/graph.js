/**
 * 结构审查·结构图：每次跑门禁把当前架构画成一张离线 HTML（仓库根 Structure.html）。
 * 目的：把「层级 / 依赖方向 / 文件头四槽」变成一眼可见的东西——节点与边的颜色只表示门禁会拦什么，不自己判好坏。
 * 管：调现有判据收事实（layers / dependency / capability / values / size / comments / unit-map），按 layers.yaml 分层排布；
 *     点能力下钻到文件级依赖，点文件看四槽与命中问题；产物每次覆盖，不进库（.gitignore 已忽略）。
 * 不管：不新增判据、不判红（永远 pass）、不改任何源码；除 Structure.html 之外不写文件。
 * 联动：tests/gates/structure/layers.js 的边与层表、run-hygiene.js 的 headerOf（四槽唯一解析器）、tests/gates/registry.js 的登记。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const L = require("./layers.js");
const dependency = require("./dependency.js");
const capability = require("./capability.js");
const values = require("./values.js");
const size = require("./size.js");
const comments = require("./comments.js");
const units = require("./map.js");

const OUT = "Structure.html";
const NODE_W = 150;
const NODE_H = 44;
const GAP_X = 14;
const BAND_GAP = 26;
/** 面板初始与回主视图时的说明；左右两处共用这一份，改文案只改这里。 */
const HINT =
  "<h2>怎么看</h2><p class=hint>边是能力之间的引用（数字为引用处数，粗细按它放大）；红色边是门禁会拦的向上/同层依赖。虚线框是扇出最大的三个单元（只回执，不设预算）。点节点下钻，点文件看目的 / 管 / 不管 / 联动。</p>";

/** 仓库相对路径（正斜杠）。 */
const relOf = (root, f) => f.slice(root.length + 1).replace(/\\/g, "/");

/** 路径所属能力；不在能力目录下返回空串。 */
function capOf(rel) {
  const m = /^src\/capabilities\/([a-z_][a-z0-9_]*)\//.exec(rel);
  return m ? m[1] : "";
}

/** 文案里出现的所有 .rs 路径（判据的锚点就是这个形状），用于把问题挂到节点上。 */
function pathsIn(text) {
  return String(text).match(/[A-Za-z0-9_./-]+\.rs(?::\d+)?/g) || [];
}

/** 现成判据的问题清单：结构图只搬运，不产生判断。 */
function judgeProblems(root) {
  const out = [];
  const push = (gate, list) => {
    for (const t of list || []) out.push({ gate: gate, text: String(t) });
  };
  push("layers", L.layers(root).problems);
  push("dependency", dependency.direction(root));
  push("capability", capability.internal(root).problems);
  push("values", values.values(root).problems);
  push("size", size.fileSize(root).problems);
  push("comments", comments.contract(root).problems);
  push("unit-map", units.unitMap(root));
  const seen = new Set();
  return out.filter((p) => {
    const k = p.gate + "\u0000" + p.text;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

/** 同能力内的文件依赖：super::m 与 crate::capabilities::<cap>::a::b 解析到真实文件。 */
function fileEdges(root, cap, rels) {
  const known = new Set(rels);
  const out = new Map();
  const add = (from, to) => {
    if (!to || to === from || !known.has(to)) return;
    const k = from + "\u0000" + to;
    const hit = out.get(k);
    if (hit) hit.refs++;
    else out.set(k, { from: from, to: to, refs: 1 });
  };
  const base = "src/capabilities/" + cap + "/";
  const resolve = (from, segs) => {
    const dir = path.posix.dirname(from);
    if (segs[0] === "super") {
      const m = segs[1];
      if (!m) return "";
      const near = dir + "/" + m + ".rs";
      if (known.has(near)) return near;
      const mod = dir + "/" + m + "/mod.rs";
      return known.has(mod) ? mod : "";
    }
    if (segs[0] !== "crate" || segs[1] !== "capabilities" || segs[2] !== cap) return "";
    const tail = segs.slice(3).filter((s) => s !== "self" && s !== "super");
    for (let n = tail.length; n >= 1; n--) {
      const stem = base + tail.slice(0, n).join("/");
      if (known.has(stem + ".rs")) return stem + ".rs";
      if (known.has(stem + "/mod.rs")) return stem + "/mod.rs";
    }
    return "";
  };
  for (const from of rels) {
    const text = dependency
      .stripTestModules(fs.readFileSync(path.join(root, from), "utf8"))
      .split(/\r?\n/)
      .filter((l) => !l.trim().startsWith("//"))
      .join("\n");
    for (const m of text.matchAll(/\bcrate(?:::[a-z_][a-z0-9_]*)+/g)) {
      const segs = m[0].split("::");
      if (segs.slice(0, 3).join("::") === "crate::capabilities::" + cap) add(from, resolve(from, segs));
    }
    for (const m of text.matchAll(/\bsuper::([a-z_][a-z0-9_]*)/g)) add(from, resolve(from, ["super", m[1]]));
  }
  return [...out.values()].sort((a, b) => (a.from < b.from ? -1 : a.from > b.from ? 1 : a.to < b.to ? -1 : 1));
}

/** 收事实：能力、层、边、文件、问题。全部排序，保证同输入同输出。 */
function collect(root) {
  const { tiers } = L.parseLayers(root);
  const dirs = L.dirsOf(root);
  const capEdges = L.edgesOf(root, dirs);
  const rows = new Map(size.scan(root).map((r) => [r.file, r]));
  const problems = judgeProblems(root);

  const caps = dirs.map((name) => {
    const layer = tiers.has(name) ? tiers.get(name) : 0;
    const rels = L.rsFilesOf(path.join(root, "src", "capabilities", name)).map((f) => relOf(root, f)).sort();
    const files = rels.map((p) => {
      const row = rows.get(p) || { prod: 0, total: 0, role: "impl" };
      const soft = row.role === "impl" ? row.prod > size.LIMITS.impl.prodSoft : row.total > size.LIMITS.support.soft;
      return { path: p, role: row.role, prod: row.prod, total: row.total, soft: soft };
    });
    return { name: name, layer: layer, out: 0, inn: 0, files: files, issues: [] };
  });
  const byName = new Map(caps.map((c) => [c.name, c]));

  const edges = [];
  for (const [key, info] of [...capEdges].sort()) {
    const [from, to] = key.split(">");
    const a = tiers.get(from);
    const b = tiers.get(to);
    edges.push({ from: from, to: to, refs: info.refs, file: relOf(root, info.file), violation: b >= a });
    if (byName.has(from)) byName.get(from).out++;
    if (byName.has(to)) byName.get(to).inn++;
  }

  const headers = require(path.join(root, "run-hygiene.js")).headers(
    caps.flatMap((c) => c.files.map((f) => f.path)),
  );
  for (const cap of caps) for (const f of cap.files) f.head = headers[f.path] || {};

  for (const p of problems) {
    const paths = pathsIn(p.text).map((x) => x.replace(/:\d+$/, ""));
    const hitFile = paths.find((x) => x.startsWith("src/capabilities/"));
    const cap = hitFile ? capOf(hitFile) : "";
    const item = { gate: p.gate, text: p.text, cap: cap, file: hitFile || "" };
    if (cap && byName.has(cap)) byName.get(cap).issues.push(item.text);
    p.cap = cap;
    p.file = hitFile || "";
  }

  const fileGraph = {};
  for (const cap of caps) {
    const rels = cap.files.map((f) => f.path);
    fileGraph[cap.name] = fileEdges(root, cap.name, rels);
  }

  const issueOf = (f) => problems.filter((p) => p.file && p.file === f).map((p) => p.text);
  for (const cap of caps) for (const f of cap.files) f.issues = issueOf(f.path);

  // 扇出不设预算：只把出边最多的三个单元标出来给人看，不进判据。
  const wide = caps.slice().sort((a, b) => b.out - a.out || a.name.localeCompare(b.name)).slice(0, 3).map((c) => c.name);

  return {
    meta: {
      wide: wide,
      caps: caps.length,
      edges: edges.length,
      files: caps.reduce((n, c) => n + c.files.length, 0),
      issues: problems.length,
      band: Math.max(...caps.map((c) => c.layer), 0),
    },
    caps: caps,
    edges: edges,
    fe: fileGraph,
    issues: problems.map((p) => ({ gate: p.gate, text: p.text, cap: p.cap, file: p.file })),
  };
}
module.exports = { collect };


/** HTML / SVG 文本转义。 */
function esc(s) {
  return String(s == null ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** 按层排布：层号大的在上（编排层在顶），同层一行居中。 */
function layout(caps) {
  const layers = [...new Set(caps.map((c) => c.layer))].sort((a, b) => b - a);
  const widest = Math.max(1, ...layers.map((l) => caps.filter((c) => c.layer === l).length));
  const width = Math.max(820, widest * (NODE_W + GAP_X) + 40);
  const pos = new Map();
  const bands = [];
  let y = 14;
  for (const layer of layers) {
    const row = caps.filter((c) => c.layer === layer).sort((a, b) => a.name.localeCompare(b.name));
    const rowW = row.length * (NODE_W + GAP_X) - GAP_X;
    const x0 = Math.max(20, (width - rowW) / 2);
    const top = y + 20;
    row.forEach((c, i) => pos.set(c.name, { x: x0 + i * (NODE_W + GAP_X), y: top, layer: layer }));
    const h = 20 + NODE_H + 16;
    bands.push({ layer: layer, y: y, h: h, n: row.length });
    y += h + BAND_GAP;
  }
  return { pos: pos, bands: bands, width: width, height: y };
}

/** 能力总图：带内铺层，边按门禁口径着色（越界红、常规灰）。 */
function capSvg(data) {
  const lay = layout(data.caps);
  const out = [];
  out.push('<svg id="cap-graph" viewBox="0 0 ' + lay.width + " " + lay.height + '" width="' + lay.width + '" height="' + lay.height + '" role="img" aria-label="能力依赖图">');
  out.push('<defs><marker id="ar" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#94a3b8"/></marker><marker id="arv" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#dc2626"/></marker></defs>');
  for (const b of lay.bands) {
    out.push('<rect x="0" y="' + b.y + '" width="' + lay.width + '" height="' + b.h + '" fill="' + (b.layer % 2 ? "#f8fafc" : "#ffffff") + '"/>');
    out.push('<text x="14" y="' + (b.y + 15) + '" class="band">第 ' + b.layer + " 层" + (b.layer === 0 ? "（kernel · 机制）" : "") + (b.n ? " · " + b.n + " 个" : " · 空") + "</text>");
  }
  for (const e of data.edges) {
    const a = lay.pos.get(e.from);
    const b = lay.pos.get(e.to);
    if (!a || !b) continue;
    const down = b.y > a.y;
    const x1 = a.x + NODE_W / 2;
    const y1 = down ? a.y + NODE_H : a.y;
    const x2 = b.x + NODE_W / 2;
    const y2 = down ? b.y : b.y + NODE_H;
    const dy = y2 - y1;
    const c1 = y1 + dy * 0.45;
    const c2 = y2 - dy * 0.45;
    const w = (0.7 + Math.min(e.refs, 12) * 0.14).toFixed(2);
    out.push('<path class="edge' + (e.violation ? " bad" : "") + '" data-from="' + esc(e.from) + '" data-to="' + esc(e.to) + '" d="M' + x1 + "," + y1 + " C" + x1 + "," + c1 + " " + x2 + "," + c2 + " " + x2 + "," + y2 + '" fill="none" stroke="' + (e.violation ? "#dc2626" : "#94a3b8") + '" stroke-width="' + w + '" stroke-opacity="' + (e.violation ? "0.95" : "0.5") + '" marker-end="url(#' + (e.violation ? "arv" : "ar") + ')"><title>' + esc(e.from + " → " + e.to + "：" + e.refs + " 处引用" + (e.violation ? "（越界）" : "")) + "</title></path>");
  }
  for (const c of data.caps) {
    const p = lay.pos.get(c.name);
    const bad = c.issues.length > 0 || data.edges.some((e) => e.violation && (e.from === c.name || e.to === c.name));
    const wide = data.meta.wide.indexOf(c.name) >= 0;
    const stroke = bad ? "#dc2626" : wide ? "#f59e0b" : "#cbd5e1";
    const dash = !bad && wide ? ' stroke-dasharray="5 4"' : "";
    out.push('<g class="cap" data-cap="' + esc(c.name) + '" transform="translate(' + p.x + "," + p.y + ')"><title>' + esc(c.name + " · 第 " + c.layer + " 层 · 出 " + c.out + " / 入 " + c.inn + " · " + c.files.length + " 个文件" + (c.issues.length ? " · " + c.issues.length + " 处命中" : "")) + '</title>');
    out.push('<rect width="' + NODE_W + '" height="' + NODE_H + '" rx="9" fill="#ffffff" stroke="' + stroke + '" stroke-width="' + (bad ? 2 : 1.2) + '"' + dash + "/>");
    out.push('<text x="12" y="19" class="nm">' + esc(c.name) + "</text>");
    out.push('<text x="12" y="34" class="sub">出 ' + c.out + " · 入 " + c.inn + " · " + c.files.length + " 文件</text>");
    if (c.issues.length) out.push('<circle cx="' + (NODE_W - 13) + '" cy="13" r="8" fill="#dc2626"/><text x="' + (NODE_W - 13) + '" y="17" class="badge" text-anchor="middle">' + (c.issues.length > 9 ? "9+" : c.issues.length) + "</text>");
    out.push("</g>");
  }
  out.push("</svg>");
  return out.join("");
}

/** 整个页面：样式与点击逻辑都内联，双击即可看（产物不进库）。 */
function page(data) {
  const capSvgText = capSvg(data);
  const m = data.meta;
  data.hint = HINT;
  const json = JSON.stringify(data).replace(/<\/script/gi, "<\\/script").replace(/</g, "\\u003c");
  const style = [
    "*{box-sizing:border-box}body{margin:0;font:14px/1.5 -apple-system,'Segoe UI','Microsoft YaHei',sans-serif;color:#0f172a;background:#f1f5f9}",
    "header{padding:14px 20px;background:#0f172a;color:#e2e8f0;display:flex;gap:18px;align-items:baseline;flex-wrap:wrap}",
    "header h1{margin:0;font-size:16px;font-weight:600;letter-spacing:.02em}",
    "header .n{font-size:12px;color:#93a3b8}header .n b{color:#fff;font-weight:600}",
    "main{display:grid;grid-template-columns:minmax(0,1fr) 380px;gap:14px;padding:14px;align-items:start;max-width:1560px;margin:0 auto}",
    "#stage svg{display:block;width:100%;height:auto;max-width:1240px;margin:0 auto}",
    "#stage{background:#fff;border:1px solid #e2e8f0;border-radius:10px;overflow:auto;padding:6px;min-width:0}",
    "#panel{background:#fff;border:1px solid #e2e8f0;border-radius:10px;padding:14px;position:sticky;top:14px;max-height:calc(100vh - 28px);overflow:auto}",
    ".band{font-size:10.5px;fill:#64748b;letter-spacing:.06em}",
    ".nm{font-size:13px;fill:#0f172a;font-weight:600}.sub{font-size:10px;fill:#64748b}",
    ".badge{font-size:9.5px;fill:#fff;font-weight:700}",
    "g.cap{cursor:pointer}g.cap:hover rect{fill:#eff6ff}",
    "path.edge{pointer-events:stroke}path.edge.hot{stroke-opacity:1;stroke:#2563eb}",
    "#crumbs{font-size:12px;color:#475569;margin:0 0 8px;display:flex;gap:6px;align-items:center}",
    "#crumbs a{color:#2563eb;cursor:pointer;text-decoration:none}#crumbs a:hover{text-decoration:underline}",
    "#panel h2{margin:0 0 6px;font-size:15px}#panel h3{margin:14px 0 6px;font-size:12px;color:#475569;letter-spacing:.04em}",
    "#panel .path{font:11.5px/1.45 ui-monospace,Consolas,monospace;color:#0f172a;background:#f8fafc;border:1px solid #e2e8f0;border-radius:6px;padding:6px;word-break:break-all}",
    "#panel dl{margin:0;font-size:12.5px}#panel dt{font-weight:600;color:#334155;margin-top:8px}#panel dd{margin:2px 0 0;color:#334155}",
    "#panel ul{margin:0;padding-left:16px;font-size:12px;color:#b91c1c}#panel ul li{margin:3px 0}",
    ".tag{display:inline-block;font-size:10.5px;padding:1px 7px;border-radius:999px;background:#e2e8f0;color:#334155;margin-right:6px}",
    ".tag.red{background:#fee2e2;color:#b91c1c}.tag.amber{background:#fef3c7;color:#92400e}",
    ".hint{font-size:12px;color:#64748b}",
    ".legend{font-size:11.5px;color:#475569;display:flex;gap:12px;flex-wrap:wrap;margin:4px 0 0}",
    ".legend i{display:inline-block;width:10px;height:10px;border-radius:3px;margin-right:4px;vertical-align:-1px}",
    "svg .fnode{cursor:pointer}svg .fnode rect{fill:#fff;stroke:#cbd5e1}svg .fnode:hover rect{fill:#eff6ff}",
    "svg .fnode.hit rect{stroke:#dc2626;stroke-width:2}",
    "svg .fline{stroke:#94a3b8;stroke-opacity:.45;fill:none}",
  ].join("");
  const java = [
    "var D=JSON.parse(document.getElementById('graph-data').textContent),HINT=D.hint;",
    "var stage=document.getElementById('stage'),panel=document.getElementById('panel'),crumbs=document.getElementById('crumbs');",
    "var home=stage.innerHTML;",
    "function esc(s){return String(s==null?'':s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/\"/g,'&quot;');}",
    "function cut(s,n){s=String(s==null?'':s);return s.length>n?s.slice(0,n-1)+'…':s;}",
    "function issues(arr){if(!arr||!arr.length)return '<p class=tag>无命中</p>';return '<ul>'+arr.map(function(t){return '<li>'+esc(t)+'</li>';}).join('')+'</ul>';}",
    "function crumb(parts){crumbs.innerHTML=parts.map(function(p,i){return i? '<span>›</span> <a data-go=\"'+esc(p[1])+'\">'+esc(p[0])+'</a>':'<a data-go=\"'+esc(p[1])+'\">'+esc(p[0])+'</a>';}).join(' ');}",
    "function home2(){stage.innerHTML=home;panel.innerHTML=HINT;crumbs.innerHTML='';wire();}",
    "function cap(name){var c=D.caps.filter(function(x){return x.name===name;})[0];if(!c)return;",
    " crumb([['全部能力',''],[c.name,c.name]]);",
    " var svg=fileGraph(c);stage.innerHTML=svg;",
    " panel.innerHTML='<h2>'+esc(c.name)+'</h2><span class=tag>第 '+c.layer+' 层</span><span class=tag>出 '+c.out+' / 入 '+c.inn+'</span><span class=tag>'+c.files.length+' 个文件</span>'+",
    "  '<h3>命中问题</h3>'+issues(c.issues)+'<h3>提示</h3><p class=hint>点文件看四槽；悬停文件高亮它的依赖。</p>';",
    " bindFile();}",
    "function fileGraph(c){var n=c.files.length,cols=Math.min(4,Math.max(1,Math.ceil(Math.sqrt(n)))),rows=Math.ceil(n/cols);",
    " var CW=300,CH=78,GX=34,GY=26,W=cols*CW+(cols-1)*GX+40,H=rows*CH+(rows-1)*GY+40;",
    " var P=new Map();c.files.forEach(function(f,i){P.set(f.path,{x:20+(i%cols)*(CW+GX),y:20+Math.floor(i/cols)*(CH+GY)});});",
    " var out=['<svg viewBox=\"0 0 '+W+' '+H+'\" width=\"'+W+'\" height=\"'+H+'\"><defs><marker id=\"fa\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto\"><path d=\"M0,0 L8,4 L0,8 z\" fill=\"#94a3b8\"/></marker></defs>'];",
    " (D.fe[c.name]||[]).forEach(function(e){var a=P.get(e.from),b=P.get(e.to);if(!a||!b)return;",
    "  var x1=a.x+CW/2,y1=a.y+56,x2=b.x+CW/2,y2=b.y;var dy=y2-y1;",
    "  out.push('<path class=fline d=\"M'+x1+','+y1+' C'+x1+','+(y1+dy*0.5)+' '+x2+','+(y2-dy*0.5)+' '+x2+','+y2+'\" stroke-width=\"'+(0.8+Math.min(e.refs,8)*0.18)+'\" marker-end=\"url(#fa)\" data-from=\"'+esc(e.from)+'\" data-to=\"'+esc(e.to)+'\"/>');});",
    " c.files.forEach(function(f){var p=P.get(f.path);var short=f.path.replace('src/capabilities/'+c.name+'/','');",
    "  out.push('<g class=\"fnode'+(f.issues&&f.issues.length?' hit':'')+'\" data-file=\"'+esc(f.path)+'\" transform=\"translate('+p.x+','+p.y+')\"><rect width=\"'+CW+'\" height=\"56\" rx=\"8\"/><text x=\"10\" y=\"18\" style=\"font-size:12px;fill:#0f172a\">'+esc(cut(short,30))+'</text><text x=\"10\" y=\"34\" style=\"font-size:10px;fill:#64748b\">'+(f.role==='test'?'测试':f.role==='support'?'测试支撑':'实现')+' · '+f.total+' 行'+(f.soft?' · 超软线':'')+'</text><text x=\"10\" y=\"48\" style=\"font-size:9.5px;fill:#94a3b8\">'+esc(cut(f.head&&f.head['目的']||'',26))+'</text></g>');});",
    " out.push('</svg>');return out.join('');}",
    "function showFile(path){var c=D.caps.filter(function(x){return x.files.some(function(f){return f.path===path;});})[0];if(!c)return;",
    " var f=c.files.filter(function(x){return x.path===path;})[0];var h=f.head||{};",
    " crumb([['全部能力',''],[c.name,c.name],[f.path.split('/').pop(),f.path]]);",
    " var dl=['目的','管','不管','联动'].map(function(k){return '<dt>'+k+'</dt><dd>'+esc(h[k]||'（缺）')+'</dd>';}).join('');",
    " panel.innerHTML='<h2>'+esc(f.path.split('/').pop())+'</h2><div class=path>'+esc(f.path)+'</div>'+",
    "  '<div style=\"margin-top:8px\"><span class=tag>'+(f.role==='test'?'测试':f.role==='support'?'测试支撑':'实现')+'</span><span class=tag>'+f.total+' 行</span>'+(f.soft?'<span class=\"tag amber\">超软线</span>':'')+(f.issues&&f.issues.length?'<span class=\"tag red\">'+f.issues.length+' 处命中</span>':'')+'</div>'+",
    "  '<h3>文件头四槽</h3><dl>'+dl+'</dl><h3>命中问题</h3>'+issues(f.issues);}",
    "function bindFile(){var nodes=stage.querySelectorAll('.fnode'),lines=stage.querySelectorAll('.fline');",
    " nodes.forEach(function(g){g.addEventListener('click',function(){showFile(g.getAttribute('data-file'));});",
    "  g.addEventListener('mouseenter',function(){var p=g.getAttribute('data-file');lines.forEach(function(l){if(l.getAttribute('data-from')===p||l.getAttribute('data-to')===p)l.setAttribute('stroke','#2563eb');});});",
    "  g.addEventListener('mouseleave',function(){lines.forEach(function(l){l.setAttribute('stroke','');});});});}",
    "function wire(){stage.querySelectorAll('g.cap').forEach(function(g){var name=g.getAttribute('data-cap');",
    " g.addEventListener('click',function(){cap(name);});",
    " g.addEventListener('mouseenter',function(){stage.querySelectorAll('path.edge').forEach(function(p){if(p.getAttribute('data-from')===name||p.getAttribute('data-to')===name)p.classList.add('hot');});});",
    " g.addEventListener('mouseleave',function(){stage.querySelectorAll('path.edge').forEach(function(p){p.classList.remove('hot');});});});}",
    "document.addEventListener('click',function(ev){var a=ev.target.closest('a[data-go]');if(!a)return;var v=a.getAttribute('data-go');if(!v){home2();}else if(v.indexOf('src/')===0){showFile(v);}else if(D.caps.some(function(c){return c.name===v;})){cap(v);} });",
    "wire();",
  ].join("\n");
  return [
    "<!doctype html>",
    '<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">',
    "<title>Solomni 结构图</title><style>" + style + "</style></head><body>",
    "<header><h1>Solomni 结构图</h1>",
    '<span class="n">能力 <b>' + m.caps + "</b> · 依赖边 <b>" + m.edges + "</b> · 源文件 <b>" + m.files + "</b> · 命中 <b>" + m.issues + "</b></span>",
    '<span class="n">由门禁 <b>structure/graph</b> 每次覆盖；层级以 layers.yaml 为准</span></header>',
    "<main><section><div id=crumbs></div><div id=stage>" + capSvgText + "</div>",
    '<div class="legend"><span><i style="background:#fff;border:1px solid #cbd5e1"></i>常规</span><span><i style="background:#fff;border:1px dashed #f59e0b"></i>扇出最大的三个</span><span><i style="background:#fff;border:1px solid #dc2626"></i>门禁命中</span><span><i style="background:#dc2626;height:2px"></i>越界依赖</span><span>点能力下钻 · 点文件看四槽</span></div></section>',
    "<aside id=panel>" + HINT + "</aside></main>",
    '<script id="graph-data" type="application/json">' + json + "</script>",
    "<script>" + java + "</script>",
    "</body></html>",
  ].join("\n");
}


/** 读上一版产物里的计数，只用于「与上次比」的差异摘要（产物本身不进库）。 */
function previousMeta(file) {
  if (!fs.existsSync(file)) return null;
  const m = /"meta":\{[^}]*\}/.exec(fs.readFileSync(file, "utf8"));
  if (!m) return null;
  try {
    return JSON.parse(m[0].slice(7));
  } catch (e) {
    return null;
  }
}

/** 计数摘要：变化量只在有上一版时给，避免把「首次生成」说成增长。 */
function summary(meta, before) {
  const base = "结构图：Structure.html " + (before ? "已刷新" : "已生成") + "（" + meta.caps + " 个能力 / " + meta.edges + " 条依赖边 / " + meta.files + " 个源文件 / " + meta.issues + " 处命中）";
  if (!before) return base;
  const parts = [];
  for (const [key, label] of [["caps", "能力"], ["edges", "依赖边"], ["files", "源文件"], ["issues", "命中"]]) {
    const d = meta[key] - (before[key] || 0);
    if (d) parts.push(label + (d > 0 ? "+" : "") + d);
  }
  return base + (parts.length ? "；比上次 " + parts.join("、") : "；与上次一致");
}

/** 生成并覆盖产物：内容不变就不写，返回值给门禁当回执。 */
function write(root) {
  const data = collect(root);
  const file = path.join(root, OUT);
  const html = page(data) + "\n";
  const before = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
  const prev = previousMeta(file);
  if (before !== html) fs.writeFileSync(file, html, "utf8");
  return { changed: before !== html, meta: data.meta, note: summary(data.meta, prev) };
}

/** 门禁步：永远 pass，只写产物与回执——结构图是「让判据可见」，不是判据本身。 */
function graph(root) {
  const r = write(root);
  return { problems: [], notes: [r.note] };
}

module.exports = { collect, page, render: (root) => page(collect(root)), write, graph };

/** 命令行：单独重画一次，便于本地看效果。 */
if (require.main === module) {
  const root = path.resolve(__dirname, "..", "..", "..");
  const r = write(root);
  console.log(r.note);
}
