/* 目的：转录的纯数据映射——一行 LineView 变渲染对象，一轮的多行合成正文 / 思维链。
 * 管：lineParts 的种类→样式映射，以及 turnWorthy / turnKey / turnText / turnReasoning 四个纯函数。
 * 不管：不碰 DOM、不读写会话状态、不发请求；同输入必得同输出（单元测试直接 import）。
 * 联动：absorb 用它把 transcript 事件变成行；渲染层用它算一轮气泡的键与正文。 */

/* [id:tag] text → 行对象（呈现即上下文：原样收录，标签做样式） */
/// 一行 → 渲染对象：**全读结构化字段**（种类 / 说话人 / 动词 / 正文），不从正文里抠标签。
/// 服务端在落行时就把身份写进字段（见 core/events.rs 的 LineView）——
/// 这里只做"种类 → 样式"的映射：系统行、用户行、轮次分隔行、模型发言、工具卡片。
function lineParts(l) {
  const text = l.line || '';
  const speaker = l.speaker || '';
  const verb = l.verb || '';
  const kind = l.kind || '';
  const reasoning = l.reasoning || null;
  // 工具行：按调用视图渲染成卡片（说话人取自视图）。
  if (l.tool || kind === 'tool') {
    // 工具行同样要有身份（卡片左上角标出是谁在调工具）——身份是这一行的，不是正文的附属。
    return [{
      cls: 'tool',
      tool: l.tool || null,
      reasoning: reasoning,
      who: (l.tool && l.tool.speaker) || speaker,
      text: '',
      speaker: (l.tool && l.tool.speaker) || speaker,
      reply: l.reply || (l.tool && l.tool.reply) || 0,
      raw: l.raw || '',
    }];
  }
  // 压缩记录：转录里的一等行（有 id，可删除/留档/恢复）；摘要 + 从哪一行起保留原文。
  if (kind === 'compacted') {
    const upTo = (l.payload && l.payload.up_to) || 0;
    return [{
      cls: 'sys compacted',
      who: '',
      text: '[压缩] 第 ' + upTo + ' 行之前已压成摘要；此后原样保留（旧内容仍可查看）：\n' + text,
      reply: l.reply || 0,
    }];
  }
  // 内容类系统提示：转录里的一等行（有 id，可回档/折叠），界面仍按系统行画。
  if (kind === 'notice') return [{ cls: 'sys notice', who: speaker, text: text }];
  // 系统行：系统注入的提醒/边界（没有说话人），或核心自己的行（代拟…）。
  if (kind === 'system' || l.system) return [{ cls: 'sys system', who: speaker, text: text }];
  // 讨论的轮次分隔行：标签是"轮次 + 号"（号在正文里，结构化）。
  if (kind === 'round') return [{ cls: 'sys system', who: '', text: '[' + speaker + ' ' + text + ']' }];
  // 用户说的行：标签在字段里（需求/开始/撤回/名单…），界面上就是"用户 + 那句话"。
  if (kind === 'user') return [{ cls: 'user', who: '用户', text: text }];
  // 模型发言：动词决定样式；"降级"是结构化信号（不在正文里找说明文案）。
  const cls = verb === 'agree' ? 'ok' : verb === 'leave' ? 'sys' : verb === 'ask' ? 'plan' : 'line';
  const deg = l.degraded === true;
  return [{
    cls: deg ? 'sys' : cls,
    who: speaker + (verb ? ' · ' + verb : '') + (deg ? ' · 信封缺失' : ''),
    text: text,
    speaker: speaker,
    verb: verb,
    degraded: deg,
    reply: l.reply || 0,
    raw: l.raw || '',
  }];
}

function turnWorthy(p) {
  return (p.cls === 'line' || p.cls === 'ok' || p.cls === 'tool') && typeof p.id === 'number';
}

function turnKey(group) {
  return group.reply ? 'R' + group.reply : 'K' + group.id;
}

function turnText(parts) {
  return parts
    .filter((p) => p.cls !== 'tool')
    .map((p) => String(p.text == null ? '' : p.text))
    .filter((t) => t.trim())
    .join('\n\n');
}

function turnReasoning(parts) {
  // reasoning 可能挂在第一条 tool 行上（模型只发工具调用、没有正文）；不能按 cls 过滤掉。
  const seen = new Set();
  const out = [];
  for (const p of parts) {
    const r = p.reasoning;
    if (!r || seen.has(r)) continue;
    seen.add(r);
    out.push(r);
  }
  return out.join('\n\n');
}

export { lineParts, turnWorthy, turnKey, turnText, turnReasoning };
