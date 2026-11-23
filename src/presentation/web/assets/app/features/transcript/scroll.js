/* 目的：转录滚动的跟随判定——纯函数，只吃尺寸数字，不碰 DOM。
 * 管：距底距离、跟随锁存该不该翻开（用户滚离底部即解除，回到底部才恢复）。
 * 不管：写 scrollTop、建浮标节点（./stream.js）、排队渲染节拍（同文件 scheduleLiveTick）。
 * 联动：stream.js 只在 #stream 的 scroll 事件里调用本模块推进锁存，渲染路径不再重判。 */

/// 距底容差：只认"真的在底部"。留几像素给分数坐标（缩放 / 高 DPI），
/// 但必须远小于用户一次轻微上滚的距离，否则就会把"用户想往回看"当成"还在底部"。
export const FOLLOW_EDGE = 12;

/// 距底还剩多少像素；滚动越界（内容比视口短、回弹）一律按 0 算。
export function bottomGap(box) {
  const gap = box.scrollHeight - box.scrollTop - box.clientHeight;
  return gap > 0 ? gap : 0;
}

/// 一次滚动之后的跟随状态：在底部就跟随，离开底部就不跟随。
/// 只在 scroll 事件里调用——渲染路径每帧重判会把容差以内的上滚立刻吃掉。
export function followAfterScroll(box) { return bottomGap(box) <= FOLLOW_EDGE; }
