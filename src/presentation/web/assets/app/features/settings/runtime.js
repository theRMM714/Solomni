/* 目的：设置中心里「生成与上下文」「沙箱围栏」「权限默认值」「资源与进程」四页的控件。
 * 管：这四页的表单字段与说明文案，读写设置值。
 * 不管：提交与保存（见 pages.js 的 settingsForm / saveSettings）、分页框架（hub.js）。
 * 联动：pages.js 的 settingsForm 逐页调用。
 */
let g, doc, api, ui, app;
let field;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ g, doc, api, ui, app } = deps);
  ({ field } = ui);
  Object.assign(root, { buildGeneratePage, buildSandboxPage, buildPermissionsPage, buildResourcesPage });
}
/// 1 页：生成与上下文。
function buildGeneratePage(c, f) {
  c.body.appendChild(f.stream.wrap);
  c.body.appendChild(f.cot.wrap);
  c.body.appendChild(field('思考等级', f.thinkLevels.node));
  c.body.appendChild(app.cfgHint('默认 off / low / high / max；可追加任意标签（如 xhigh、高）。标签就是档位，模型登记按它写"标签 → 映射值"；会话状态栏切档时只列这组模型都登记了的标签。'));
  c.body.appendChild(f.to.wrap);
  c.body.appendChild(app.cfgHint('这是“等第一个响应/第一个数据块”的预算，不是整次流的总时长：正常流式只要还在出数据就不会超时；流开始后 120 秒没有新数据才判定断流。用尽时会中断本轮并提示，点「继续」可重试（会话不会作废）。'));
  c.body.appendChild(f.remind.wrap);
  c.body.appendChild(app.cfgHint('调用次数没有上限：模型继续核实就继续跑，直到它给出表态（你可以随时点「停止」）。'));
  c.body.appendChild(app.cfgHint('一轮内提醒到顶就记一行「未回应」放过它，整轮继续（不阻塞）。'));
  c.body.appendChild(f.pct.wrap);
  c.body.appendChild(app.cfgHint('压缩由 AI 自己做：到点自动压一次；也可以随时手动 /compact。填 0 = 不自动压。'));
  c.body.appendChild(f.keep.wrap);
  c.body.appendChild(app.cfgHint('按模型窗口算：压缩时至少保留最近这么多内容，更早的压成摘要；填 0 = 只留摘要。建议小于上面的压缩阈值。'));
  c.body.appendChild(field('压缩用模型', f.compactModel));
  c.body.appendChild(app.cfgHint('默认与会话同模型，保住前缀缓存；也可以选登记处里的另一个模型专门做总结。只在压缩触发时解析，不按回合、不刷新会话。'));
}

/// 2 页：沙箱围栏（全局硬约束）。
function buildSandboxPage(c, f) {
  c.body.appendChild(app.cfgHint('以下沙箱策略是全局硬约束：会话不能改；保存后自动整体重启。', 'warn'));
  c.body.appendChild(f.network.wrap);
  c.body.appendChild(field('文件系统围栏', f.enFs));
  c.body.appendChild(field('网络围栏（仅在网络关闭时需要）', f.enNet));
  c.body.appendChild(field('进程树围栏', f.enTree));
  c.body.appendChild(field('环境白名单', f.enEnv));
  c.body.appendChild(field('机制不可用时', f.onUnavailable));
  c.body.appendChild(f.writeAcl.wrap);
  c.body.appendChild(app.cfgHint('Windows 上 require 文件系统/网络围栏必须先勾选「允许 Windows 写本机目录 ACL」；否则保存会被拒绝。要无围栏请把对应 enforce 降为 best_effort。', 'warn'));
}

/// 3 页：权限默认值。
function buildPermissionsPage(c, f) {
  c.body.appendChild(f.hostRoots.f);
  c.body.appendChild(f.hardDeny.f);
  c.body.appendChild(app.cfgHint('默认权限是新建会话的起点；全局黑名单是硬底线，会话只能追加、不能清空。'));
  c.body.appendChild(field('默认权限档位', f.defProfile));
  c.body.appendChild(f.defSharedRead.wrap);
  c.body.appendChild(f.defSharedWrite.wrap);
  c.body.appendChild(f.defTools.f);
}

/// 4 页：资源与进程。
function buildResourcesPage(c, f) {
  c.body.appendChild(f.guests.wrap);
  c.body.appendChild(app.cfgHint('每台虚拟机固定 2 GB 内存 / 2 核：到上限时新建会如实拒绝，不会悄悄抢资源。填 0 = 不限制。'));
  const title = doc.createElement('div');
  title.className = 'wf-label';
  title.textContent = '现在在跑什么';
  const box = doc.createElement('div');
  box.className = 'cfg-log';
  box.textContent = '读取中…';
  c.body.appendChild(title);
  c.body.appendChild(box);
  api('GET', '/api/processes').then((r) => {
    const list = (r && r.procs) || [];
    box.textContent = list.length ? list.map((p) =>
      (p.alive ? '在跑' : '已退出（等对账清）') + ' · ' + p.purpose + ' · ' + p.owner + ' · pid ' + p.pid +
      (p.orphanRisk ? ' · 父死子亡未挂上（核心被强杀会留下）：' + (p.guardNote || '') : '')
    ).join('\n') : '没有产品起的长驻进程。';
  }).catch((e) => { box.textContent = '读不到：' + e.message; });
}