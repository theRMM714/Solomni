/**
 * 环境引导自检：\`env.js\` 的 MinGW 引导链——下载、续传、结构判定、sha256 闸与显式接受开关。
 * 管：用**真实子进程**跑 \`<临时根>/env.js setup --yes\`，配一个本地 HTTP 服务器按需应答（206 / 不认 Range / 中途断开）。
 * 不管：Rust 侧 provision 的校验（在 \`provision\` 的单元用例里）、dev.js 的子命令表（在 \`tests/dev-loop.mjs\`）。
 * 判据：见 \`docs/testing/execution-ci.md\` 的「环境引导自检」。dlltool 只在 Windows 那条路上，别的平台如实跳过。
 * 约束：只写 \`target/test-scratch/\`（不入库）；不碰真实 \`.tools/\`、不碰外网、不起 Rust 安装。
 */

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRATCH = path.join(ROOT, "target", "test-scratch", "env-bootstrap");
const IS_WIN = process.platform === "win32";
const MB = 1024 * 1024;

// ---------- 夹具：一个结构合法、体积过 10MB 闸、含 mingw64/bin/{dlltool,as}.exe 的 zip ----------
function crc32(buf) {
  const table = new Int32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[i] = c;
  }
  let c = ~0;
  for (const b of buf) c = table[(c ^ b) & 0xff] ^ (c >>> 8);
  return ~c >>> 0;
}

/** 目的：纯 JS 造一个 store-only 的 zip（跨平台，不靠 zip 命令）。 */
function makeZip(entries) {
  const chunks = [];
  const central = [];
  let offset = 0;
  for (const e of entries) {
    const name = Buffer.from(e.name, "utf8");
    const crc = crc32(e.data);
    const lh = Buffer.alloc(30);
    lh.writeUInt32LE(0x04034b50, 0);
    lh.writeUInt16LE(20, 4);
    lh.writeUInt16LE(0x0800, 6);
    lh.writeUInt16LE(0, 8);
    lh.writeUInt16LE(0, 10);
    lh.writeUInt16LE(0x21, 12);
    lh.writeUInt32LE(crc, 14);
    lh.writeUInt32LE(e.data.length, 18);
    lh.writeUInt32LE(e.data.length, 22);
    lh.writeUInt16LE(name.length, 26);
    lh.writeUInt16LE(0, 28);
    chunks.push(lh, name, e.data);
    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE(20, 4);
    cd.writeUInt16LE(20, 6);
    cd.writeUInt16LE(0x0800, 8);
    cd.writeUInt16LE(0, 10);
    cd.writeUInt16LE(0, 12);
    cd.writeUInt16LE(0x21, 14);
    cd.writeUInt32LE(crc, 16);
    cd.writeUInt32LE(e.data.length, 20);
    cd.writeUInt32LE(e.data.length, 24);
    cd.writeUInt16LE(name.length, 28);
    cd.writeUInt16LE(0, 30);
    cd.writeUInt16LE(0, 32);
    cd.writeUInt16LE(0, 34);
    cd.writeUInt16LE(0, 36);
    cd.writeUInt32LE(0, 38);
    cd.writeUInt32LE(offset, 42);
    central.push(Buffer.concat([cd, name]));
    offset += lh.length + name.length + e.data.length;
  }
  const cdBuf = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(0, 4);
  eocd.writeUInt16LE(0, 6);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(cdBuf.length, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20);
  return Buffer.concat([...chunks, cdBuf, eocd]);
}

const FIXTURE = makeZip([
  { name: "mingw64/bin/dlltool.exe", data: Buffer.from("fake-dlltool") },
  { name: "mingw64/bin/as.exe", data: Buffer.from("fake-as") },
  // 体积过 10MB 闸：store-only，多大都不压缩（内容无所谓）。
  { name: "mingw64/padding.bin", data: Buffer.alloc(12 * MB) },
]);
const FIXTURE_SHA = crypto.createHash("sha256").update(FIXTURE).digest("hex");
const GARBAGE = Buffer.concat([Buffer.from("<html>not an archive</html>"), Buffer.alloc(12 * MB)]);

/** 目的：本地应答源——按 case 的 mode 模拟三种服务端（支持续传 / 不认 Range / 传到一半断开）。 */
function startServer(mode) {
  const requests = [];
  const server = http.createServer((req, res) => {
    const range = req.headers.range || null;
    requests.push({ url: req.url, range });
    const isGarbage = req.url.includes("garbage");
    const body = isGarbage ? GARBAGE : FIXTURE;
    // 探测请求一定带 Range（env.js 用 -r 0-2097151 测速）：它必须成功，否则"没有可用源"会盖住要测的那条路。
    const m = /^bytes=(\d+)-$/.exec(range || "");
    if (mode === "drop" && !range) {
      res.writeHead(200, { "Content-Length": body.length });
      res.write(body.subarray(0, MB));
      setTimeout(() => res.destroy(), 10);
      return;
    }
    if (mode === "norange") {
      res.writeHead(200, { "Content-Length": body.length });
      res.end(body);
      return;
    }
    if (m) {
      const start = Number(m[1]);
      const slice = body.subarray(start);
      res.writeHead(206, {
        "Content-Range": "bytes " + start + "-" + (body.length - 1) + "/" + body.length,
        "Content-Length": slice.length,
      });
      res.end(slice);
      return;
    }
    res.writeHead(200, { "Content-Length": body.length });
    res.end(body);
  });
  return new Promise((done) => {
    server.listen(0, "127.0.0.1", () => {
      done({ server, requests, base: "http://127.0.0.1:" + server.address().port });
    });
  });
}

/** 目的：跑一次 \`env.js setup --yes\`（输出落文件：管道捕获在受限环境里不可靠，文件三平台都行）。 */
function runSetup(dir, env) {
  return new Promise((resolve, reject) => {
    const logPath = path.join(dir, "run.log");
    const fd = fs.openSync(logPath, "w");
    const child = spawn(process.execPath, ["env.js", "setup", "--yes"], {
      cwd: dir,
      env,
      stdio: ["ignore", fd, fd],
    });
    const timer = setTimeout(() => {
      try { child.kill(); } catch {}
      reject(new Error("env.js setup 超时（60s）"));
    }, 60000);
    child.on("error", (e) => {
      clearTimeout(timer);
      fs.closeSync(fd);
      reject(e);
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      fs.closeSync(fd);
      resolve({ code, out: fs.readFileSync(logPath, "utf8") });
    });
  });
}

async function runCase(c) {
  const dir = path.join(SCRATCH, c.name);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, ".tools"), { recursive: true });
  fs.copyFileSync(path.join(ROOT, "env.js"), path.join(dir, "env.js"));
  // 假 cargo：env.js 只查它在不在（也就不会去装 Rust、不碰网络）。
  fs.writeFileSync(path.join(dir, "fake-cargo"), "");
  const { server, requests, base } = await startServer(c.mode);
  const url = base + (c.path || "/winlibs.zip");
  try {
    fs.writeFileSync(
      path.join(dir, "links.yaml"),
      "winlibs.upstream.releases: https://example.test/releases\n" +
        "winlibs.upstream.asset: " + url + "\n" +
        "winlibs.upstream.asset.sha256: " + (c.sha256 || FIXTURE_SHA) + "\n",
    );
    if (c.prepare) c.prepare(path.join(dir, ".tools"), url);
    const env = { ...process.env, SOLOMNI_CARGO: path.join(dir, "fake-cargo") };
    // 这两个开关不能从调用者的环境漏进来（否则会跳过要测的那条路）。
    delete env.SOLOMNI_MINGW_BIN;
    delete env.SOLOMNI_ACCEPT_ASSET_CHANGE;
    Object.assign(env, c.env || {});
    const r = await runSetup(dir, env);
    c.check({ dir, out: r.out, code: r.code, requests, url });
    fs.rmSync(dir, { recursive: true, force: true });
    console.log("PASS " + c.name);
  } catch (e) {
    const log = fs.existsSync(path.join(dir, "run.log")) ? fs.readFileSync(path.join(dir, "run.log"), "utf8") : "";
    console.error("FAIL " + c.name + "：" + e.message);
    console.error("  现场保留在 " + path.relative(ROOT, dir));
    console.error("  输出尾部：\n" + log.split(/\r?\n/).slice(-12).join("\n"));
    throw e;
  } finally {
    server.close();
  }
}

const tools = (dir) => path.join(dir, ".tools");
const installed = (dir) => fs.existsSync(path.join(tools(dir), "mingw64", "bin", "dlltool.exe"));
const keptZip = (dir) => fs.existsSync(path.join(tools(dir), "winlibs.zip"));
const keptMarker = (dir) => fs.existsSync(path.join(tools(dir), "winlibs.zip.src"));

const tail = (out) => "\n" + out.split(/\r?\n/).slice(-10).join("\n");

const CASES = [
  {
    name: "full-download-then-verify-and-extract",
    mode: "range",
    check: ({ dir, out, code }) => {
      assert.equal(code, 0, "整份下载要成功并装好：" + tail(out));
      assert.match(out, /sha256 verified/, "要过完整性闸");
      assert.ok(installed(dir), "要解出 .tools/mingw64/bin/dlltool.exe");
      assert.ok(!keptZip(dir), "装完要删掉归档");
      assert.ok(!keptMarker(dir), "装完要删掉来源标记");
    },
  },
  {
    name: "resume-uses-the-recorded-source-and-ranges",
    mode: "range",
    prepare: (t, url) => {
      fs.writeFileSync(path.join(t, "winlibs.zip"), FIXTURE.subarray(0, MB + 123));
      fs.writeFileSync(path.join(t, "winlibs.zip.src"), url);
    },
    check: ({ dir, out, code, requests }) => {
      assert.match(out, /will resume/, "分片要真的续传（不是直接进解压）");
      assert.match(out, /resuming from the same source/, "续传要打向记录下来的那个源");
      assert.ok(
        requests.some((r) => r.range === "bytes=" + (MB + 123) + "-"),
        "续传要带 Range 从分片末尾开始：" + JSON.stringify(requests.map((r) => r.range)),
      );
      assert.equal(code, 0, "续传完成后要装好：" + tail(out));
      assert.ok(installed(dir), "续传完成后要解出 dlltool.exe");
    },
  },
  {
    name: "partial-without-source-restarts-from-scratch",
    mode: "range",
    prepare: (t) => {
      fs.writeFileSync(path.join(t, "winlibs.zip"), FIXTURE.subarray(0, MB + 123));
    },
    check: ({ dir, out, code, requests }) => {
      assert.match(out, /partial has no recorded source/, "来源不明要从头下");
      const beyondZero = requests.filter((r) => r.range && !/^bytes=0-/.test(r.range));
      assert.deepEqual(beyondZero, [], "不该带着未知来源续传：" + JSON.stringify(beyondZero));
      assert.equal(code, 0, "从头下要能完成：" + tail(out));
      assert.ok(installed(dir), "要解出 dlltool.exe");
    },
  },
  {
    name: "structurally-broken-archive-is-deleted-and-reported",
    mode: "range",
    path: "/garbage.bin",
    check: ({ dir, out, code }) => {
      assert.equal(code, 1, "结构不对要如实失败");
      assert.match(out, /归档结构不对/, "要说明是结构不对：" + tail(out));
      assert.ok(!keptZip(dir), "结构不对的归档要删掉");
      assert.ok(!installed(dir), "不该装出任何东西");
    },
  },
  {
    name: "sha-mismatch-keeps-the-file-and-names-both-values",
    mode: "range",
    sha256: "0".repeat(64),
    prepare: (t) => {
      fs.writeFileSync(path.join(t, "winlibs.zip"), FIXTURE);
    },
    check: ({ dir, out, code, url }) => {
      assert.equal(code, 1, "哈希不符要拒绝装");
      assert.match(out, /sha256 不符/, "要报哈希不符");
      assert.ok(out.includes("0".repeat(64)), "要报台账值");
      assert.ok(out.includes(FIXTURE_SHA), "要报实际值");
      assert.ok(out.includes(url), "要报来源");
      assert.ok(keptZip(dir), "不许删文件（删了会在 pin 过期时变成重下死循环）");
      assert.ok(!installed(dir), "不符的归档不准解压");
    },
  },
  {
    name: "accept-switch-is-the-only-way-past-a-mismatch",
    mode: "range",
    sha256: "0".repeat(64),
    env: { SOLOMNI_ACCEPT_ASSET_CHANGE: "1" },
    prepare: (t) => {
      fs.writeFileSync(path.join(t, "winlibs.zip"), FIXTURE);
    },
    check: ({ dir, out, code }) => {
      assert.equal(code, 0, "显式接受后要能继续：" + tail(out));
      assert.match(out, /接受/, "要如实说这次是按开关接受处理的");
      assert.ok(installed(dir), "接受后要解出 dlltool.exe");
    },
  },
  {
    name: "server-that-ignores-ranges-keeps-the-partial",
    mode: "norange",
    prepare: (t, url) => {
      fs.writeFileSync(path.join(t, "winlibs.zip"), FIXTURE.subarray(0, MB + 123));
      fs.writeFileSync(path.join(t, "winlibs.zip.src"), url);
    },
    check: ({ dir, out, code }) => {
      assert.equal(code, 1, "源不认续传要如实失败");
      assert.match(out, /下载没有完成/, "要说明下载没完成：" + tail(out));
      assert.ok(keptZip(dir), "分片要留着，下次接着续");
      assert.ok(keptMarker(dir), "来源标记也要留着（否则下次会从头下）");
      assert.ok(!installed(dir), "没下完不准解压");
    },
  },
  {
    name: "connection-dropped-midway-keeps-the-partial",
    mode: "drop",
    check: ({ dir, out, code }) => {
      assert.equal(code, 1, "传到一半断开要如实失败");
      assert.match(out, /下载没有完成/, "要说明下载没完成：" + tail(out));
      assert.ok(keptZip(dir), "断开的半截要留着续传");
      assert.ok(keptMarker(dir), "来源标记要留着");
      assert.ok(!installed(dir), "没下完不准解压");
    },
  },
];


if (!IS_WIN) {
  // dlltool 只在 Windows 那条路上：这里如实跳过，不静默当作通过。
  console.log("[探针] env.js 的 MinGW 引导只在 Windows 上存在：本平台跳过环境引导自检");
  console.log("ENV-BOOTSTRAP-OK");
} else {
  fs.rmSync(SCRATCH, { recursive: true, force: true });
  for (const c of CASES) await runCase(c);
  console.log("ENV-BOOTSTRAP-OK");
}
