/* Solomni 转录中心：极简 Markdown 渲染（no-build、无依赖）。
 * 安全底线：一律先转义再拼标签，链接只放行 http/https；永不把模型原文当 HTML 注入。
 * 支持：围栏代码块（含语言标注，yaml/json 等）、标题、列表、表格、引用、分隔线、
 *       行内代码、粗体、斜体、删除线、链接；段落内单换行按 <br> 保留（聊天场景更合用）。
 */
(function (root) {
  'use strict';

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"]/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c];
    });
  }

  // 行内：先扣下行内代码，再处理链接/粗斜体，最后把代码放回。
  function inline(s) {
    var codes = [];
    var out = esc(s).replace(/\u0060([^\u0060]+)\u0060/g, function (_, c) {
      codes.push(c);
      return '\u0000' + (codes.length - 1) + '\u0000';
    });
    out = out.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, function (_, t, u) {
      return '<a href="' + u + '" target="_blank" rel="noopener noreferrer">' + t + '</a>';
    });
    out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    out = out.replace(/__([^_]+)__/g, '<strong>$1</strong>');
    out = out.replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>');
    out = out.replace(/~~([^~]+)~~/g, '<del>$1</del>');
    out = out.replace(/\u0000(\d+)\u0000/g, function (_, i) {
      return '<code>' + codes[Number(i)] + '</code>';
    });
    return out;
  }

  function isBlank(l) { return /^\s*$/.test(l); }
  function isFence(l) { return /^\s*\u0060\u0060\u0060/.test(l); }
  function isHeading(l) { return /^#{1,6}\s+/.test(l); }
  function isQuote(l) { return /^\s*>\s?/.test(l); }
  function isItem(l) { return /^\s*([-*+]|\d+\.)\s+/.test(l); }

  function cells(line) {
    return line.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map(function (c) { return c.trim(); });
  }

  function markdownToHtml(src) {
    var lines = String(src == null ? '' : src).replace(/\r\n?/g, '\n').split('\n');
    var html = '';
    var i = 0;
    while (i < lines.length) {
      var line = lines[i];
      var fence = line.match(/^\s*\u0060\u0060\u0060\s*([\w+#.-]*)\s*$/);
      if (fence) {
        var lang = fence[1] || '';
        var buf = [];
        i++;
        while (i < lines.length && !/^\s*\u0060\u0060\u0060\s*$/.test(lines[i])) { buf.push(lines[i]); i++; }
        i++;
        html += '<pre class="md-pre"><code' + (lang ? ' class="lang-' + esc(lang) + '"' : '') + '>' + esc(buf.join('\n')) + '</code></pre>';
        continue;
      }
      var h = line.match(/^(#{1,6})\s+(.*)$/);
      if (h) {
        var n = h[1].length;
        html += '<h' + n + '>' + inline(h[2]) + '</h' + n + '>';
        i++;
        continue;
      }
      if (/^\s*([-*_])\s*(\1\s*){2,}$/.test(line)) { html += '<hr>'; i++; continue; }
      if (isQuote(line)) {
        var q = [];
        while (i < lines.length && isQuote(lines[i])) { q.push(lines[i].replace(/^\s*>\s?/, '')); i++; }
        html += '<blockquote>' + q.map(inline).join('<br>') + '</blockquote>';
        continue;
      }
      if (line.indexOf('|') >= 0 && i + 1 < lines.length && /^\s*\|?\s*:?-{2,}/.test(lines[i + 1])) {
        var head = cells(line);
        i += 2;
        var body = [];
        while (i < lines.length && lines[i].indexOf('|') >= 0) { body.push(cells(lines[i])); i++; }
        html += '<table class="md-table"><thead><tr>' + head.map(function (c) { return '<th>' + inline(c) + '</th>'; }).join('')
          + '</tr></thead><tbody>' + body.map(function (r) {
            return '<tr>' + r.map(function (c) { return '<td>' + inline(c) + '</td>'; }).join('') + '</tr>';
          }).join('') + '</tbody></table>';
        continue;
      }
      if (isItem(line)) {
        var ordered = /^\s*\d+\.\s+/.test(line);
        var items = [];
        while (i < lines.length && isItem(lines[i])) {
          var text = lines[i].replace(/^\s*([-*+]|\d+\.)\s+/, '');
          i++;
          while (i < lines.length && !isBlank(lines[i]) && !isItem(lines[i]) && !isFence(lines[i]) && !isHeading(lines[i]) && !isQuote(lines[i])) {
            text += '\n' + lines[i].trim();
            i++;
          }
          items.push('<li>' + inline(text).replace(/\n/g, '<br>') + '</li>');
        }
        html += (ordered ? '<ol>' : '<ul>') + items.join('') + (ordered ? '</ol>' : '</ul>');
        continue;
      }
      if (isBlank(line)) { i++; continue; }
      var p = [];
      while (i < lines.length && !isBlank(lines[i]) && !isFence(lines[i]) && !isHeading(lines[i]) && !isQuote(lines[i]) && !isItem(lines[i])) {
        p.push(lines[i]); i++;
      }
      html += '<p>' + inline(p.join('\n')).replace(/\n/g, '<br>') + '</p>';
    }
    return html;
  }

  root.markdownToHtml = markdownToHtml;
})(typeof globalThis !== 'undefined' ? globalThis : this);
