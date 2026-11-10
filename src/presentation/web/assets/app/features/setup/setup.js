/* 目的：依赖补齐：一键执行 setup 动作，并把命令输出与诊断翻译成人话。
 * 管：补齐弹层的进度展示与轮询。
 * 不管：动作清单与执行（后端 /api/actions）、弹层栈（app.js）。
 * 联动：app.js 的横幅与向导调用 openSetupModal / setupAction。
 */
let g, doc, api, ui, app;
let btn;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ g, doc, api, ui, app } = deps);
  ({ btn } = ui);
  Object.assign(root, { setupAction, openSetupModal, diagnosisText });
}
/* ---------- 依赖补齐：把"缺什么"变成"点一下装上" ----------
 * 判据全在后端：`/api/setup/status` 的 `pending` 决定装什么（界面不自己挑条目——挑错了会把已装的重下一遍）。
 * 这里只做两步确认（先看清单、再下载）与进度；装完即生效，不需要重启。
 * 进度必须**看得见**：作业一起就进进度视图（当前步骤 + 滚动日志 + 取消）；关掉窗再打开也接着显示正在跑的那一次。 */
function setupAction(before, onDone) {
  const b = btn('补齐虚拟机依赖…', 'btn btn-block');
  b.onclick = () => {
    try {
      if (before) before();
      openSetupModal(onDone);
    } catch (e) { app.notice('打开补齐弹层失败', e.message, 'err'); }
  };
  return b;
}

function openSetupModal(onDone) {
  app.openModal('补齐虚拟机依赖', (c) => {
    const box = doc.createElement('div'); box.className = 'cfg';
    c.body.appendChild(box);
    const log = doc.createElement('div'); log.className = 'cfg-log';
    c.body.appendChild(log);
    const finish = () => { c.close(); if (onDone) onDone(); };

    // 依赖状态 → 三种视图：已在跑（进度）/ 要装什么（清单）/ 都齐了。
    async function load() {
      box.innerHTML = '';
      log.textContent = '';
      box.appendChild(ui.hint('正在读本机的依赖状态…'));
      let st = null;
      try { st = await api('GET', '/api/setup/status'); }
      catch (e) {
        box.innerHTML = '';
        box.appendChild(ui.hint('读不到依赖状态：' + e.message, 'err'));
        return;
      }
      box.innerHTML = '';
      const job = st.job || {};
      // 关掉窗又打开：这一次还在跑就接着显示它，而不是再起一个（后端会拒绝第二个作业）。
      if (job.running) { watch(st); return; }
      const pending = Array.isArray(st.pending) ? st.pending : null;
      if (!pending) {
        box.appendChild(ui.hint('这个进程没有给出要补哪些条目（界面与核心版本不一致）：重启应用后再试。', 'err'));
        const close = btn('关闭', 'btn btn-block'); close.onclick = finish;
        box.appendChild(close);
        return;
      }
      if (!pending.length) {
        box.appendChild(ui.hint('依赖都已在场：不需要补齐。', 'ok'));
        const close = btn('关闭', 'btn btn-block'); close.onclick = finish;
        box.appendChild(close);
        return;
      }
      const deps = (st.deps || []).filter((d) => pending.indexOf(d.id) >= 0);
      box.appendChild(ui.div('cfg-sec-title', '要装 ' + deps.length + ' 条（只写项目内，不动系统）'));
      let total = 0;
      for (const d of deps) {
        total += d.size_mb || 0;
        const line = doc.createElement('div');
        line.className = 'cfg-line';
        line.textContent = '· ' + d.id + (d.size_mb ? '（约 ' + d.size_mb + ' MB）' : '') + (d.note ? '：' + d.note : '');
        box.appendChild(line);
        if (d.url) { const u = doc.createElement('div'); u.className = 'cfg-dir'; u.textContent = d.url; box.appendChild(u); }
      }
      box.appendChild(ui.hint('合计约 ' + total + ' MB：从网络下载后解包进项目内的 .vm/。'));
      const go = btn('开始下载安装', 'btn btn-primary btn-block');
      go.onclick = () => start(pending, go);
      box.appendChild(go);
      const later = btn('取消', 'btn btn-block'); later.onclick = c.close;
      box.appendChild(later);
    }

    async function start(ids, go) {
      go.disabled = true;
      c.setMsg('正在起补齐作业…');
      try { await api('POST', '/api/setup/ensure', { deps: ids, confirm: true }); }
      catch (e) { go.disabled = false; c.setMsg('起作业失败：' + e.message, true); return; }
      c.setMsg('作业已开始：正在下载并解包（进度见下面）');
      watch({ job: { seq: 0, running: true } });
    }

    // 进度视图：当前步骤 + 滚动日志 + 取消。轮询是长连接（服务端挂到有新行或 20s 到点）。
    function watch(st) {
      box.innerHTML = '';
      log.textContent = '';
      box.appendChild(ui.div('cfg-sec-title', '正在补齐（只写项目内）'));
      const step = ui.hint('等待第一条进度…');
      box.appendChild(step);
      const cancel = btn('取消作业', 'btn btn-block');
      cancel.onclick = async () => {
        try { await api('DELETE', '/api/setup/job'); c.setMsg('已请求取消，等作业收尾…'); }
        catch (e) { c.setMsg('取消失败：' + e.message, true); }
      };
      box.appendChild(cancel);
      let since = (st.job && st.job.seq) || 0;
      (async function poll() {
        for (;;) {
          let now = null;
          try { now = await api('GET', '/api/setup/status?since=' + since + '&wait=1'); }
          catch (e) { c.setMsg('读进度失败：' + e.message, true); return; }
          const job = now.job || {};
          if ((job.seq || 0) > since) since = job.seq;
          const lines = job.lines || [];
          log.textContent = lines.join('\n');
          if (lines.length) step.textContent = '现在：' + lines[lines.length - 1];
          if (!job.running) {
            if (job.error) {
              c.setMsg('补齐失败：' + job.error, true);
              step.textContent = '失败：' + job.error;
              return;
            }
            done(now);
            return;
          }
          c.setMsg('正在补齐…（可以先关掉这个窗，作业会继续跑；再打开还能接着看）');
        }
      })();
    }

    function done(st) {
      const left = Array.isArray(st.pending) ? st.pending : [];
      c.setMsg(left.length ? '作业结束了，但还有没装上的：' + left.join('、') : '补齐完成：依赖都齐了');
      box.innerHTML = '';
      const note = doc.createElement('div'); note.className = 'cfg-hint ok';
      // 生效时机由后端保证（补齐完即按新状态判定，下一次起 guest 就用新的共享档）：这里只说结论。
      note.textContent = '已经生效：再打开配置 / 向导就按新状态判定，下一次启动虚拟机就用上，不需要重启。';
      box.appendChild(note);
      const again = btn('关闭', 'btn btn-block'); again.onclick = finish;
      box.appendChild(again);
    }

    load();
  }, true);
}

/// 结构化诊断 → 中文（呈现层文案，不进提示词册）。
function diagnosisText(d) {
  if (!d || typeof d !== 'object') return String(d);
  if (d.Missing) {
    return '模块 ' + d.Missing.module + ' 需要的运行能力 ' + d.Missing.capability +
      ' 在本档位没有：把它放进依赖文件夹后点「重新扫描」，或确认基础工具链 / 宿主 PATH。';
  }
  if (d.Ambiguous) {
    return '运行能力 ' + d.Ambiguous.capability + ' 有多个版本（' + (d.Ambiguous.versions || []).join('、') +
      '）：在下面「定版」里选一个版本，工具才跑得起来。';
  }
  if (d.UnknownPin) {
    return '运行能力 ' + d.UnknownPin.capability + ' 定的版本 ' + d.UnknownPin.version +
      ' 不在本档位可用的包里：改一个版本，或把该版本的运行包放进依赖文件夹后点「重新扫描」。';
  }
  if (d.Conflict) {
    return '运行包 ' + d.Conflict.a + ' 与 ' + d.Conflict.b + ' 都要写进 ' + d.Conflict.path +
      '：装配会互相覆盖，换版本或去掉其中一个。';
  }
  if (d.NotForTarget) {
    return '模块 ' + d.NotForTarget.module + ' 需要的运行能力 ' + d.NotForTarget.capability +
      ' 在本档位用不了：包库里只有 ' + (d.NotForTarget.packages || []).join('、') +
      '（目标：' + d.NotForTarget.target + '）。换一个包，或改用虚拟机档。';
  }
  if (d.NoBin) {
    return '运行包 ' + d.NoBin.package + ' 没有可执行的 bin/：本机档放不进 PATH。检查包内容或换一个包。';
  }
  if (d.Path) {
    return '运行包 PATH 合成失败：' + d.Path.why;
  }
  if (d.MissingEntry) {
    return '模块 ' + d.MissingEntry.module + ' 的工具 ' + d.MissingEntry.tool +
      ' 入口不在场：' + d.MissingEntry.command + '。先按模块 README 或 --build 编一次。';
  }
  return JSON.stringify(d);
}