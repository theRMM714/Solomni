#!/usr/bin/env node
'use strict';

/*
 * render.report —— 把 corpus.jsonl 渲染成一份自包含、可离线打开的 HTML 报告。
 *
 * 协议：参数从 stdin 收 JSON（UTF-8）；结果写 stdout；失败写 stderr 并以非零退出码结束。
 * 依赖：只用 node 标准库（fs / path / readline），无任何第三方依赖。
 * 安全：语料内容一律经 HTML 转义后进入文本节点；HTML 内联 CSS 与内联 SVG，不引用任何外部资源。
 */

const fs = require('fs');
const path = require('path');
const readline = require('readline');

const EXIT_USAGE = 2;
const EXIT_ERROR = 1;
const DEFAULT_TITLE = '资料报告';
const EXCERPT_CHARS = 200;
const BROKEN_PREVIEW = 5;
const COLORS = ['#4f8ef7', '#37b679', '#f2a33c', '#e2605c', '#8b6ff0', '#2fa8b5', '#c46bd8', '#8d8d8d'];

function fail(message, code) {
  process.stderr.write('render.report: ' + message + '\n');
  process.exit(typeof code === 'number' ? code : EXIT_ERROR);
}

function usageFail(message) {
  process.stderr.write('render.report: ' + message + '\n');
  process.stderr.write('用法: echo \'{"corpus":"...","out":"...","index":"...","title":"..."}\' | node tools/report.js\n');
  process.exit(EXIT_USAGE);
}

/* 先转义 & 再转义其余字符；双引号与单引号都转义，保证内容永远是文本而不是标签。 */
function esc(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function readStdin() {
  return new Promise(function (resolve, reject) {
    const chunks = [];
    process.stdin.on('data', function (chunk) {
      chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
    });
    process.stdin.on('end', function () {
      resolve(Buffer.concat(chunks).toString('utf8'));
    });
    process.stdin.on('error', reject);
  });
}

function requireString(args, key) {
  const value = args[key];
  if (typeof value !== 'string' || value.trim() === '') {
    usageFail('缺少必填参数 ' + key + '（需要非空字符串）');
  }
  return value;
}

function optionalString(args, key) {
  const value = args[key];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string' || value.trim() === '') {
    usageFail('可选参数 ' + key + ' 若提供就必须是非空字符串');
  }
  return value;
}

function toPosix(value) {
  return String(value).replace(/\\/g, '/');
}

function decodeLoose(value) {
  try {
    return decodeURIComponent(value);
  } catch (err) {
    return value;
  }
}

/* 语料里 rel 的规范化形式：统一分隔符、尽量解码、去掉 ./ 前缀、消解 ..。 */
function normalizeRel(value) {
  let out = toPosix(String(value));
  out = decodeLoose(out);
  out = path.posix.normalize(out);
  if (out === '.') out = '';
  if (out.indexOf('./') === 0) out = out.slice(2);
  return out;
}

function firstChars(text, limit) {
  const points = Array.from(text);
  if (points.length <= limit) return { text: text, truncated: false };
  return { text: points.slice(0, limit).join(''), truncated: true };
}

function normalizeDoc(raw) {
  const text = typeof raw.text === 'string' ? raw.text : '';
  const headings = Array.isArray(raw.headings) ? raw.headings.filter(function (h) { return typeof h === 'string'; }) : [];
  const links = Array.isArray(raw.links) ? raw.links.filter(function (l) { return typeof l === 'string'; }) : [];
  const kind = typeof raw.kind === 'string' && raw.kind !== '' ? raw.kind : 'unknown';
  const chars = Number.isFinite(raw.chars) ? raw.chars : Array.from(text).length;
  const lines = Number.isFinite(raw.lines) ? raw.lines : (text === '' ? 0 : text.split('\n').length);
  const bytes = Number.isFinite(raw.bytes) ? raw.bytes : null;
  const excerpt = firstChars(text, EXCERPT_CHARS);
  return {
    rel: raw.rel,
    path: typeof raw.path === 'string' ? raw.path : '',
    kind: kind,
    bytes: bytes,
    chars: chars,
    lines: lines,
    headings: headings,
    links: links,
    excerpt: excerpt.text,
    excerptTruncated: excerpt.truncated
  };
}

/* 流式读 corpus.jsonl：逐行解析，坏行只计数不中断；正文只留前 200 字，不整份驻留内存。 */
async function readCorpus(file) {
  const docs = [];
  const malformed = [];
  const relSet = new Set();
  const stream = fs.createReadStream(file, { encoding: 'utf8' });
  const rl = readline.createInterface({ input: stream, crlfDelay: Infinity });
  let lineNo = 0;
  for await (const raw of rl) {
    lineNo += 1;
    const line = raw.replace(/^\uFEFF/, '').trim();
    if (line === '') continue;
    let parsed;
    try {
      parsed = JSON.parse(line);
    } catch (err) {
      malformed.push({ line: lineNo, reason: 'JSON 解析失败：' + (err && err.message ? err.message : String(err)) });
      continue;
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || typeof parsed.rel !== 'string' || parsed.rel.trim() === '') {
      malformed.push({ line: lineNo, reason: '不是 JSON 对象或缺少非空字符串字段 rel' });
      continue;
    }
    const doc = normalizeDoc(parsed);
    docs.push(doc);
    relSet.add(normalizeRel(doc.rel));
  }
  return { docs: docs, malformed: malformed, relSet: relSet };
}

/* 相对引用（RFC 3986）：不以 # 开头、不带 URI scheme、不以 // 开头。http/https 与 #锚点 一律不检查。 */
function isRelativeLink(link) {
  const value = String(link).trim();
  if (value === '') return false;
  if (value.charAt(0) === '#') return false;
  if (/^\/\//.test(value)) return false;
  if (/^[A-Za-z][A-Za-z0-9+.\-]*:/.test(value)) return false;
  return true;
}

/* 相对链接相对于“所在文档的目录”解析，同时也接受直接以 rel 为目标的写法。 */
function linkCandidates(rel, link) {
  let target = String(link).split('#')[0].split('?')[0];
  if (target === '') return [];
  target = decodeLoose(target);
  target = toPosix(target);
  const base = normalizeRel(rel);
  const dir = path.posix.dirname(base);
  const out = [];
  const seen = new Set();
  function push(value) {
    if (value === '') return;
    let norm = path.posix.normalize(toPosix(value));
    if (norm.indexOf('./') === 0) norm = norm.slice(2);
    if (norm.charAt(0) === '/') norm = norm.slice(1);
    if (norm === '' || norm === '.') return;
    if (!seen.has(norm)) {
      seen.add(norm);
      out.push(norm);
    }
  }
  push(dir === '.' || dir === '' ? target : path.posix.join(dir, target));
  push(target);
  return out;
}

function checkLinks(docs, relSet) {
  const broken = [];
  const seen = new Set();
  let checked = 0;
  docs.forEach(function (doc) {
    doc.links.forEach(function (link) {
      if (!isRelativeLink(link)) return;
      checked += 1;
      const candidates = linkCandidates(doc.rel, link);
      if (candidates.length === 0) return;
      const found = candidates.some(function (candidate) { return relSet.has(candidate); });
      if (found) return;
      const key = normalizeRel(doc.rel) + ' -> ' + link;
      if (seen.has(key)) return;
      seen.add(key);
      broken.push({ from: doc.rel, link: link, candidates: candidates });
    });
  });
  return { checked: checked, broken: broken };
}

function kindStats(docs) {
  const map = new Map();
  docs.forEach(function (doc) {
    let entry = map.get(doc.kind);
    if (!entry) {
      entry = { kind: doc.kind, count: 0, chars: 0 };
      map.set(doc.kind, entry);
    }
    entry.count += 1;
    entry.chars += doc.chars || 0;
  });
  return Array.from(map.values()).sort(function (a, b) {
    if (b.count !== a.count) return b.count - a.count;
    return a.kind < b.kind ? -1 : a.kind > b.kind ? 1 : 0;
  });
}

/* 内联 SVG 条形图：不写 xmlns，HTML 解析器按 SVG 命名空间处理，产物里不出现任何 URL。 */
function svgChart(stats) {
  if (stats.length === 0) return '<p class="empty">（没有可统计的文档）</p>';
  const rowH = 30;
  const padTop = 8;
  const labelW = 110;
  const barMax = 380;
  const valueW = 70;
  const width = labelW + barMax + valueW;
  const height = padTop * 2 + rowH * stats.length;
  const max = Math.max.apply(null, stats.map(function (s) { return s.count; }));
  const parts = [];
  parts.push('<svg class="chart" width="' + width + '" height="' + height + '" viewBox="0 0 ' + width + ' ' + height + '" role="img" aria-label="按 kind 的文档数条形图">');
  stats.forEach(function (stat, index) {
    const y = padTop + index * rowH;
    const barW = Math.max(2, Math.round((stat.count / max) * barMax));
    parts.push('<text x="' + (labelW - 8) + '" y="' + (y + 20) + '" text-anchor="end" class="svg-label">' + esc(stat.kind) + '</text>');
    parts.push('<rect x="' + labelW + '" y="' + (y + 4) + '" width="' + barW + '" height="20" rx="3" fill="' + COLORS[index % COLORS.length] + '"></rect>');
    parts.push('<text x="' + (labelW + barW + 8) + '" y="' + (y + 20) + '" class="svg-value">' + esc(stat.count) + '</text>');
  });
  parts.push('</svg>');
  return parts.join('\n');
}

const CSS = [
  ':root{color-scheme:light dark}',
  'body{margin:0;padding:24px;background:#f6f7f9;color:#1d2129;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI","Microsoft YaHei",sans-serif;line-height:1.6}',
  'main{max-width:980px;margin:0 auto}',
  'h1{font-size:1.7rem;margin:0 0 4px}',
  'h2{font-size:1.25rem;margin:28px 0 12px;padding-bottom:6px;border-bottom:2px solid #dfe3e8}',
  'h3{font-size:1.05rem;margin:0 0 6px}',
  'h4{font-size:.9rem;margin:12px 0 4px;color:#4e5969}',
  '.meta{color:#4e5969;font-size:.85rem;margin:0 0 8px}',
  '.card{background:#fff;border:1px solid #e3e6eb;border-radius:8px;padding:16px;margin-bottom:14px}',
  '.stats{list-style:none;display:flex;flex-wrap:wrap;gap:20px;padding:0;margin:0}',
  '.stats b{font-size:1.25rem;display:block}',
  'table{border-collapse:collapse;width:100%;font-size:.9rem}',
  'th,td{border:1px solid #e3e6eb;padding:6px 10px;text-align:left;vertical-align:top}',
  'th{background:#f1f3f5}',
  '.chart{max-width:100%;height:auto;overflow:visible}',
  '.svg-label{font-size:13px;fill:#1d2129}',
  '.svg-value{font-size:13px;fill:#4e5969}',
  'code{background:#f1f3f5;padding:1px 5px;border-radius:4px;font-size:.85em;word-break:break-all}',
  '.badge{font-size:.75rem;background:#e8f1fe;color:#1c5cd6;border-radius:10px;padding:1px 8px;margin-left:6px}',
  '.metrics{color:#4e5969;font-size:.85rem;margin:0 0 6px}',
  '.excerpt{white-space:pre-wrap;background:#fbfcfd;border:1px dashed #e3e6eb;border-radius:6px;padding:8px 10px;margin:0;font-size:.9rem}',
  '.outline{margin:0;padding-left:20px;font-size:.9rem}',
  '.broken{color:#c0392b;font-weight:600}',
  '.ok{color:#1f8a4c;font-weight:600}',
  '.empty{color:#86909c}',
  'footer{margin-top:28px;color:#86909c;font-size:.8rem}',
  '@media (prefers-color-scheme:dark){body{background:#14161a;color:#e6e8eb}.card{background:#1c1f24;border-color:#2b2f36}h2{border-color:#2b2f36}th,td{border-color:#2b2f36}th{background:#23272d}code{background:#23272d}.badge{background:#1d2b47;color:#9dc0ff}.excerpt{background:#191c21;border-color:#2b2f36}.meta,.metrics,.svg-value{color:#a9b0ba}.svg-label{fill:#e6e8eb}}'
].join('\n');

function buildHtml(model) {
  const docs = model.docs;
  const stats = model.stats;
  const parts = [];
  parts.push('<!DOCTYPE html>');
  parts.push('<html lang="zh-CN">');
  parts.push('<head>');
  parts.push('<meta charset="utf-8">');
  parts.push('<meta name="viewport" content="width=device-width, initial-scale=1">');
  parts.push('<title>' + esc(model.title) + '</title>');
  parts.push('<style>');
  parts.push(CSS);
  parts.push('</style>');
  parts.push('</head>');
  parts.push('<body>');
  parts.push('<main>');
  parts.push('<header>');
  parts.push('<h1>' + esc(model.title) + '</h1>');
  parts.push('<p class="meta">生成时间 ' + esc(model.generatedAt) + ' · 自包含单文件报告，无任何外部资源</p>');
  parts.push('</header>');

  parts.push('<section id="overview">');
  parts.push('<h2>概览</h2>');
  parts.push('<div class="card">');
  parts.push('<ul class="stats">');
  parts.push('<li>文档数<b>' + esc(docs.length) + '</b></li>');
  parts.push('<li>总字数<b>' + esc(model.totalChars) + '</b></li>');
  parts.push('<li>相对链接<b>' + esc(model.linkCheck.checked) + '</b></li>');
  parts.push('<li>坏链<b>' + esc(model.linkCheck.broken.length) + '</b></li>');
  if (model.malformed.length > 0) {
    parts.push('<li>跳过的行<b>' + esc(model.malformed.length) + '</b></li>');
  }
  parts.push('</ul>');
  if (model.malformed.length > 0) {
    parts.push('<p class="broken">跳过了 ' + esc(model.malformed.length) + ' 行：JSON 非法或缺少 rel 字段，已跳过，未计入统计。</p>');
  }
  parts.push('</div>');
  if (stats.length > 0) {
    parts.push('<div class="card">');
    parts.push('<h3>按 kind 统计</h3>');
    parts.push('<table>');
    parts.push('<thead><tr><th>kind</th><th>文档数</th><th>字数</th></tr></thead>');
    parts.push('<tbody>');
    stats.forEach(function (stat) {
      parts.push('<tr><td>' + esc(stat.kind) + '</td><td>' + esc(stat.count) + '</td><td>' + esc(stat.chars) + '</td></tr>');
    });
    parts.push('</tbody>');
    parts.push('</table>');
    parts.push('<h4>文档数条形图</h4>');
    parts.push(svgChart(stats));
    parts.push('</div>');
  }
  parts.push('</section>');

  parts.push('<section id="toc">');
  parts.push('<h2>目录</h2>');
  if (docs.length === 0) {
    parts.push('<p class="empty">（语料里没有文档）</p>');
  }
  docs.forEach(function (doc, index) {
    parts.push('<article class="card" id="doc-' + (index + 1) + '">');
    parts.push('<h3>' + esc(index + 1) + '. <code>' + esc(doc.rel) + '</code><span class="badge">' + esc(doc.kind) + '</span></h3>');
    parts.push('<p class="metrics">' + esc(doc.chars) + ' 字符 · ' + esc(doc.lines) + ' 行' + (doc.bytes === null ? '' : ' · ' + esc(doc.bytes) + ' 字节') + '</p>');
    parts.push('<h4>标题大纲</h4>');
    if (doc.headings.length === 0) {
      parts.push('<p class="empty">（无标题）</p>');
    } else {
      parts.push('<ol class="outline">');
      doc.headings.forEach(function (heading) {
        parts.push('<li>' + esc(heading) + '</li>');
      });
      parts.push('</ol>');
    }
    parts.push('<h4>摘要（正文前 ' + esc(EXCERPT_CHARS) + ' 字）</h4>');
    if (doc.excerpt === '') {
      parts.push('<p class="empty">（正文为空）</p>');
    } else {
      parts.push('<p class="excerpt">' + esc(doc.excerpt) + (doc.excerptTruncated ? '…' : '') + '</p>');
    }
    parts.push('</article>');
  });
  parts.push('</section>');

  parts.push('<section id="links">');
  parts.push('<h2>链接检查</h2>');
  parts.push('<div class="card">');
  parts.push('<p>共检查 ' + esc(model.linkCheck.checked) + ' 条相对链接（http/https、// 开头与 #锚点 不计入），发现坏链 ' + esc(model.linkCheck.broken.length) + ' 条。</p>');
  if (model.linkCheck.broken.length === 0) {
    parts.push('<p class="ok">未发现坏链。</p>');
  } else {
    parts.push('<table>');
    parts.push('<thead><tr><th>来源文档</th><th>链接</th><th>解析目标</th><th>说明</th></tr></thead>');
    parts.push('<tbody>');
    model.linkCheck.broken.forEach(function (item) {
      parts.push('<tr><td><code>' + esc(item.from) + '</code></td><td><code>' + esc(item.link) + '</code></td><td><code>' + esc(item.candidates.join(' | ')) + '</code></td><td>在语料里找不到目标</td></tr>');
    });
    parts.push('</tbody>');
    parts.push('</table>');
  }
  parts.push('</div>');
  parts.push('</section>');

  parts.push('<footer>由 render.report 生成，仅消费 corpus.jsonl；内容全部经 HTML 转义。</footer>');
  parts.push('</main>');
  parts.push('</body>');
  parts.push('</html>');
  return parts.join('\n') + '\n';
}

async function main() {
  const raw = await readStdin();
  const text = raw.replace(/^\uFEFF/, '').trim();
  if (text === '') usageFail('stdin 为空，需要一个 JSON 参数对象');
  let args;
  try {
    args = JSON.parse(text);
  } catch (err) {
    usageFail('stdin 不是合法 JSON：' + (err && err.message ? err.message : String(err)));
  }
  if (!args || typeof args !== 'object' || Array.isArray(args)) usageFail('stdin 必须是 JSON 对象');

  const corpusPath = requireString(args, 'corpus');
  const outPath = requireString(args, 'out');
  const indexPath = optionalString(args, 'index');
  const title = optionalString(args, 'title') || DEFAULT_TITLE;

  let result;
  try {
    result = await readCorpus(corpusPath);
  } catch (err) {
    fail('无法读取语料 ' + corpusPath + '：' + (err && err.message ? err.message : String(err)));
  }

  const docs = result.docs;
  const stats = kindStats(docs);
  const totalChars = docs.reduce(function (sum, doc) { return sum + (doc.chars || 0); }, 0);
  const linkCheck = checkLinks(docs, result.relSet);
  const generatedAt = new Date().toISOString();

  const html = buildHtml({
    title: title,
    generatedAt: generatedAt,
    docs: docs,
    stats: stats,
    totalChars: totalChars,
    malformed: result.malformed,
    linkCheck: linkCheck
  });

  try {
    fs.writeFileSync(outPath, html, 'utf8');
  } catch (err) {
    fail('无法写入 HTML 产物 ' + outPath + '：' + (err && err.message ? err.message : String(err)));
  }

  let indexWritten = null;
  if (indexPath !== undefined) {
    const list = docs.map(function (doc) {
      return { rel: doc.rel, kind: doc.kind, chars: doc.chars, headings: doc.headings };
    });
    try {
      fs.writeFileSync(indexPath, JSON.stringify(list, null, 2) + '\n', 'utf8');
      indexWritten = path.resolve(indexPath);
    } catch (err) {
      fail('无法写入索引产物 ' + indexPath + '：' + (err && err.message ? err.message : String(err)));
    }
  }

  const byKind = stats.map(function (stat) { return stat.kind + '=' + stat.count; }).join(', ') || '（无）';
  const lines = [];
  lines.push('render.report 完成');
  lines.push('文档数: ' + docs.length);
  lines.push('总字数: ' + totalChars);
  lines.push('按 kind: ' + byKind);
  lines.push('跳过了 ' + result.malformed.length + ' 行（JSON 非法或缺少 rel）');
  lines.push('相对链接: ' + linkCheck.checked + ' 条；坏链: ' + linkCheck.broken.length + ' 条');
  linkCheck.broken.slice(0, BROKEN_PREVIEW).forEach(function (item) {
    lines.push('  - ' + item.from + ' -> ' + item.link);
  });
  if (linkCheck.broken.length > BROKEN_PREVIEW) {
    lines.push('  …还有 ' + (linkCheck.broken.length - BROKEN_PREVIEW) + ' 条坏链，详见报告链接检查段');
  }
  lines.push('HTML: ' + path.resolve(outPath));
  if (indexWritten !== null) lines.push('索引: ' + indexWritten);
  process.stdout.write(lines.join('\n') + '\n');
}

main().catch(function (err) {
  fail('未预期的错误：' + (err && err.stack ? err.stack : String(err)));
});
