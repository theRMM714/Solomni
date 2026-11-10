/* 目的：设置中心的长说明文案，按功能集中存放（纯数据，不含逻辑）。
 * 管：第一层只给决定所需的一句话；原理与影响范围放 detail，安全与不可逆内容放 safety。
 * 不管：控件与提交（见同目录 runtime.js / pages.js / storage.js）。
 * 联动：由 runtime.js / pages.js / storage.js 引用，经 ui.detail 渲染。 */

export const settingsCopy = {
  thinkingLevels: {
    detail: '默认 off / low / high / max；可追加任意标签（如 xhigh、高）。标签就是档位，模型登记按它写「标签 → 映射值」；会话状态栏切档时只列这组模型都登记了的标签。',
  },
  firstResponseTimeout: {
    detail: '这是「等第一个响应 / 第一个数据块」的预算，不是整次流的总时长：正常流式只要还在出数据就不会超时；流开始后 120 秒没有新数据才判定断流。用尽时会中断本轮并提示，点「继续」可重试（会话不会作废）。',
  },
  remindCap: {
    detail: '调用次数没有上限：模型继续核实就继续跑，直到它给出表态（你可以随时点「停止」）。一轮内提醒到顶就记一行「未回应」放过它，整轮继续（不阻塞）。',
  },
  compactAt: {
    detail: '压缩由 AI 自己做：到点自动压一次；也可以随时手动 /compact。填 0 = 不自动压。',
  },
  compactKeep: {
    detail: '按模型窗口算：压缩时至少保留最近这么多内容，更早的压成摘要；填 0 = 只留摘要。建议小于上面的压缩阈值。',
  },
  compactModel: {
    detail: '默认与会话同模型，保住前缀缓存；也可以选登记处里的另一个模型专门做总结。只在压缩触发时解析，不按回合、不刷新会话。',
  },
  sandboxHard: {
    safety: '沙箱策略是全局硬约束：会话不能改；保存后自动整体重启。',
    safetyKind: 'warn',
  },
  windowsAcl: {
    safety: 'Windows 上启用文件系统 / 网络围栏，必须先勾选「允许 Windows 写本机目录 ACL」；否则保存会被拒绝。要无围栏，请把对应 enforce 降为 best_effort。',
    safetyKind: 'warn',
  },
  permissionDefaults: {
    detail: '默认权限是新建会话的起点；全局黑名单是硬底线，会话只能追加、不能清空。',
  },
  guestLimits: {
    detail: '每台虚拟机固定 2 GB 内存 / 2 核：到上限时新建会如实拒绝，不会悄悄抢资源。填 0 = 不限制。',
  },
  thinkingMapping: {
    detail: '字符串 = {"reasoning_effort": 值} 简写；{...} = 原样合并进请求体（保留键会被拒）。每行的值都不能为空。',
  },
  storageBudget: {
    detail: '上限只约束可回收缓存（构建缓存 / 下载缓存 / 过期可写层 / 日志），不约束会话、虚拟根与模块产物。到上限先回收一次，不拒绝构建；全局回收默认且至少 15 分钟一次，关闭自动回收后只剩手动。',
  },
  storageRestore: {
    safety: '还原虚拟根会删除 guest 内未落宿主的数据，并用清单重建；不备份、不可撤销。',
    safetyKind: 'err',
  },
  storageGc: {
    detail: '按策略回收只处理已超上限或超过保留天数的项；未到条件时不会删任何东西。要手动清理，用下方单项或「清空全部缓存」。',
  },
  storageEmpty: {
    detail: '当前没有达到策略条件的项：未超上限，也没有超过保留天数的缓存；可用单项清理或「清空全部缓存」。',
  },
};
