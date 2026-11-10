/* 目的：转录的行级渲染部件——Markdown 容器、折叠 store、工具卡片、长路径胶囊、单行/说明行。
 * 管：mdNode / appendBody / fold* / flushScroll / toolDetail / toolCard / buildRootList / shortPath /
 *   reasoningBlock / rewindButton / lineBlock / sessionNote。
 * 不管：不决定渲染哪些行、不管整帧与增量策略（见 ./stream.js 与 ./page.js）。
 * 联动：折叠与滚动位置存在会话上（fold / scroll）；长路径根来自 state.pathRoots（由向导写入）。 */

let g, doc, state, stores, api, ui, app;

/// 思维链块：永远默认折叠，点击（原生 details）才展开。
/// Markdown 容器：有渲染器就渲染，没有就退回纯文本（安全）。
function mdNode(text) {
  const d = doc.createElement('div');
  d.className = 'md';
  if (typeof g.markdownToHtml === 'function') d.innerHTML = shortPath(g.markdownToHtml(text));
  else d.textContent = text;
  return d;
}

/// 正文：AI/用户发言按 Markdown 渲染，系统提示保持纯文本。
function appendBody(el, cls, text) {
  // 正文必须**追加**，不能 el.innerHTML = …：那会把上面刚挂的身份标题（.who）一起抹掉，
  // 于是 agree（绿框）这类非 line 类的消息就"没了说话人"——用户不知道这句来自谁。
  if (cls === 'line' || cls === 'plan' || cls === 'user') {
    el.appendChild(mdNode(text));
    return;
  }
  const body = doc.createElement('div');
  body.className = 'body';
  body.innerHTML = shortPath(app.escHtml(text)); // 系统/验收等行：纯文本转义后同样缩写长路径
  el.appendChild(body);
}

/* 折叠状态：会话级 store（key → bool）。renderStream 每次全量重建 DOM，
 * 所以折叠状态必须由「稳定键 + 会话级 store」自己保存与恢复——键只依赖"只追加"的顺序，
 * 刷新多少次都不变。CoT 与工具卡片的三处折叠共用同一个 store。 */
function foldOpen(sess, key) {
  if (!sess.fold) sess.fold = {};
  return !!sess.fold[key];
}

/// 绑定折叠：按 store 设初值，并在 toggle 时回写。
function foldBind(sess, key, d) {
  d.open = foldOpen(sess, key); // 先设初值再挂监听，避免程序化赋值触发一次回写
  d.addEventListener('toggle', () => {
    sess.fold = sess.fold || {};
    sess.fold[key] = d.open;
  });
  return d;
}

/* <pre> 的滚动位置也要自己存/恢复：定稿整帧重建时，不能把用户拖到一半的滚动条弹回去。
 * 键沿用折叠键 + ':scroll'；恢复必须在节点插入文档之后做（离屏元素的 scrollTop 在浏览器里不生效），
 * 所以这里只登记，由 flushScroll 在渲染收尾时统一写回。 */
let pendingScroll = [];

function scrollKey(key) { return key + ':scroll'; }

function flushScroll(sess) {
  for (const it of pendingScroll) {
    const v = sess.scroll && sess.scroll[scrollKey(it.key)];
    if (typeof v === 'number' && v > 0) it.pre.scrollTop = v;
  }
  pendingScroll = [];
}

/// 折叠的原始内容块：默认收起，点开才看（参数 / 结果 / 原文）。
/// literal = true 时逐字保留（「原文」是要给用户照着复制的记录，不做长路径缩写）。
function toolDetail(label, text, sess, key, literal) {
  const d = doc.createElement('details'); d.className = 'tool-raw';
  foldBind(sess, key, d);
  const s = doc.createElement('summary'); s.textContent = label;
  const pre = doc.createElement('pre'); pre.className = 'tool-pre';
  // 先转义再缩写：文字内容不变，只是把长路径显示成短式胶囊（「原文」保持逐字）。
  const shown = app.escHtml(text == null || text === '' ? '（空）' : text);
  pre.innerHTML = literal ? shown : shortPath(shown);
  const sk = scrollKey(key);
  pre.addEventListener('scroll', () => { sess.scroll = sess.scroll || {}; sess.scroll[sk] = pre.scrollTop; });
  pendingScroll.push({ pre: pre, key: key });
  d.appendChild(s); d.appendChild(pre);
  return d;
}

/// 工具调用卡片：头部只写「模块.工具名」+ 成败；参数与结果折叠在里面（折叠状态走 fold store）。
/// 流式 tool_call 与权威 transcript 的 tool 行共用这一个渲染函数，也共用同一个折叠键。
function toolCard(t, sess, key, who) {
  const info = t || {};
  const el = doc.createElement('div');
  el.className = 'line tool-card ' + (info.ok ? 'ok' : 'bad');
  el.title = info.speaker ? info.speaker + ' 调用工具' : '工具调用';
  // 左上角先标出**是谁**在调工具：与其它行同一条规则（每个 kind 都带身份）。
  const speaker = who || info.speaker || '';
  if (speaker) {
    const w = doc.createElement('span'); w.className = 'who'; w.textContent = speaker; el.appendChild(w);
  }
  const head = doc.createElement('div'); head.className = 'tool-head';
  const name = doc.createElement('span'); name.className = 'tool-name';
  name.textContent = (info.module ? info.module + '.' : '') + (info.name || '工具');
  const mark = doc.createElement('span'); mark.className = 'tool-mark'; mark.textContent = info.ok ? '✓' : '✗';
  head.appendChild(name); head.appendChild(mark);
  el.appendChild(head);
  el.appendChild(toolDetail('参数', info.args, sess, key + ':args'));
  el.appendChild(toolDetail('结果', info.output, sess, key + ':output'));
  return el;
}

/* 长路径缩写：转录里是真实绝对路径（如 D:/…/session/总结/work/README.md），显示时缩成短式胶囊：
 * 落在共享区根之下 → work/相对路径；落在某 agent 沙箱根之下 → 沙箱 <agent>/相对路径；
 * 根还没拿到、或不在任何根之内 → **原样显示**（不做任何猜测性缩短）。完整路径放在 title 里。
 *
 * 安全前提（为什么可以对这个 HTML 字符串做替换）：md.js 只把 http/https 渲染成 href 且已转义，
 * 所以文件路径不可能出现在 HTML 属性里；字符类里排除 & 是为了不被 &quot; 这类实体的分号吞掉。 */

/// 从 /files 的 roots 构造匹配表；没有 roots（后端未提供 / 还没拉到）→ null = 原样显示。
function buildRootList(roots) {
  if (!roots) return null;
  const out = [];
  const add = (root, kind, name) => {
    const r = String(root == null ? '' : root).replace(/\/+$/, '');
    if (r) out.push({ root: r, kind: kind, name: name || '' });
  };
  for (const a of (roots.agents || [])) add(a && a.root, 'agent', a && a.name);
  add(roots.work, 'work');
  // 先长后短：避免父根遮住子根（agent 根先入列，同长时优先）
  out.sort((x, y) => y.root.length - x.root.length);
  return out.length ? out : null;
}

/// 把正文 HTML 里的长路径替换成短式胶囊（完整路径进 title）。
function shortPath(html) {
  const src = String(html == null ? '' : html);
  const list = state.pathRoots;
  if (!list || !list.length) return src;
  const alt = list.map((it) => app.escapeRe(it.root)).join('|');
  const re = new RegExp('(?:' + alt + ')/([^\\s<>\\x22\\x27&]*)', 'g');
  return src.replace(re, (m, rel, offset, whole) => {
    // 前面还是路径字符 → 只是一个更长路径的中间段，不动它
    const before = offset > 0 ? whole[offset - 1] : '';
    if (before && /[A-Za-z0-9_\-./\\]/.test(before)) return m;
    for (const it of list) {
      if (m.indexOf(it.root + '/') === 0) {
        const label = it.kind === 'agent' ? '沙箱 ' + it.name + '/' + rel : 'work/' + rel;
        return '<span class="ref-pill" title="' + app.escHtml(m) + '">' + app.escHtml(label) + '</span>';
      }
    }
    return m;
  });
}

/// 思维链块：永远默认折叠；展开状态记在会话的 fold store 里，流式重建也不会被收回。
function reasoningBlock(text, sess, key) {
  const d = doc.createElement('details');
  d.className = 'cot';
  foldBind(sess, key, d);
  const s = doc.createElement('summary'); s.textContent = '思维链';
  const b = doc.createElement('div'); b.className = 'cot-body'; b.textContent = text;
  d.appendChild(s); d.appendChild(b);
  return d;
}

/// 回档按钮：回到这一行（留档或删除，点开后再选）。图标 + 原生 title（长说明不会被滚动裁掉）。
function rewindButton(id) {
  const b = ui.iconButton({
    icon: 'undo',
    label: '回档到此处',
    tip: '',
    cls: 'line-act',
    onClick: () => app.rewindTo(id),
  });
  b.title = '回到这一行：留档（可恢复）或删除（不可恢复）';
  return b;
}

/// **唯一的建块函数**：一行（渲染部件）→ 一个节点。身份在上、思维链次之、正文按 kind；工具行走卡片。
/// 定稿行与实时块都从这里出——"两处各建一次"正是身份时有时无、空块漏出来的来源。
function lineBlock(p, s, key, live) {
  if (p.tool) {
    const card = toolCard(p.tool, s, key, p.who || p.speaker);
    if (p.reasoning && state.settings.show_reasoning) card.appendChild(reasoningBlock(p.reasoning, s, key));
    return { node: card, txt: null, cot: null };
  }
  const cls = p.cls || 'line';
  const el = doc.createElement('div');
  el.className = 'line ' + cls + (live ? ' streaming' : '');
  if (p.who) {
    const w = doc.createElement('span'); w.className = 'who'; w.textContent = p.who; el.appendChild(w);
  }
  // 思维链在回答之上（先想后说）；默认折叠，展开状态由 fold store 记住（键 = 行/块的稳定键）。
  let cot = null;
  if (p.reasoning && state.settings.show_reasoning) {
    cot = reasoningBlock(p.reasoning, s, key);
    el.appendChild(cot);
  }
  let txt = null;
  if (String(p.text == null ? '' : p.text).length) {
    appendBody(el, cls, p.text);
    txt = el.children[el.children.length - 1] || null;
  }
  return { node: el, txt: txt, cot: cot };
}

/// 定稿行渲染到容器里（每次全量重建这个容器；折叠与 <pre> 滚动状态由 store 恢复）。
/// 会话级说明（代理身份 / 已停止 / 已关闭）：放转录最上方，**不冒充任何一方的发言**。
/// 它只说这条会话是什么、还该不该被驱动——正文一律来自转录本身。
function sessionNote(s) {
  if (!s) return null;
  const bits = [];
  if (s.mode === 'proxy') {
    bits.push('代理模式：核心代你挑人、建子工作并回答它们的关卡——这就是全权；' +
      '按「停止」会让相关会话一起停下。');
  }
  if (s.run === 'stopped') bits.push('这条会话已停止：不会再被派发或唤醒；启用后按会话状态继续。');
  if (s.run === 'closed') bits.push('这条会话已关闭：终态，不会再被派发或唤醒。');
  if (s.readonly) bits.push('历史回放为只读：点「启用会话」后才能发送消息。');
  if (!bits.length) return null;
  const d = doc.createElement('div');
  d.className = 'agent-note';
  d.textContent = bits.join('\n');
  return d;
}

export { mdNode, appendBody, foldOpen, foldBind, scrollKey, flushScroll, toolDetail, toolCard, buildRootList, shortPath, reasoningBlock, rewindButton, lineBlock, sessionNote, pendingScroll };

/// 挂载：装配依赖并把本模块的入口交给组装点。
export function mount(root, deps) {
  ({ g, doc, state, stores, api, ui, app } = deps);
  Object.assign(root, { mdNode, appendBody, foldOpen, foldBind, scrollKey, flushScroll, toolDetail, toolCard, buildRootList, shortPath, reasoningBlock, rewindButton, lineBlock, sessionNote, pendingScroll });
}
