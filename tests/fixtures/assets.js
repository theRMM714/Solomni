// e2e 通用资产 fixture：真实 1×1 PNG 与最小 MP4（ftyp 盒），只用于 L4 的共享区资产旅程。
// 说明：MP4 只需有合法 ftyp 盒，L4 验证的是字节资产契约，不是播放器兼容性。
"use strict";
module.exports = {
  PNG_1X1_BASE64:
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  MP4_FTYP_HEX: "00000018667479706d703432000000006d70343269736f6d",
};
