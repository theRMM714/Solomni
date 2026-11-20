/**
 * 环境引导自检：\`env.js\` 的 MinGW（Windows）与 Zig cc（Linux / macOS）引导链。
 * 管：用**真实子进程**跑 \`<临时根>/env.js setup --yes\`，配一个本地 HTTP 服务器按需应答（206 / 不认 Range / 中途断开）；
 *     借用工具的"能构建"门槛（\`probeBuild\`）、POSIX 的"宿主 cc 可用就不下载 / 缺失才自举"、
 *     \`--target=\` 翻译、\`node env.js\` 的自诊断输出、工具链注入判据（active.json → PATH 前置 / nodeBin / 在场事实）、
 *     python/node 安装链与跨语言身份钉住，以及入口 re-exec（项目内 node 接管 / 守卫 / 回退）。
 * 不管：Rust 侧 provision 的校验（在 \`provision\` 的单元用例里）、dev.js 的子命令表（在 \`tests/dev-loop.mjs\`）。
 * 判据：见 \`docs/testing/execution-ci.md\` 的「环境引导自检」。dlltool 只在 Windows；Zig 只在 POSIX。
 * 约束：只写 \`target/test-scratch/\`（不入库）；不碰真实 \`.tools/\`、不碰外网、不起 Rust 安装。
 */

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRATCH = path.join(ROOT, "target", "test-scratch", "env-bootstrap");
const IS_WIN = process.platform === "win32";
const IS_MAC = process.platform === "darwin";
const MB = 1024 * 1024;
// 与 env.js 同一套宿主目标写法；POSIX 夹具只在 Linux / macOS 跑。
const ARCH = process.arch === "arm64" ? "aarch64" : "x86_64";
const TARGET = (IS_MAC ? "macos-" : "linux-") + ARCH;
const HOST = IS_MAC ? ARCH + "-apple-darwin" : ARCH + "-unknown-linux-gnu";
const CC_VERSION = "9.9.9";
const CC_INSTALL = ".tools/toolchains/" + TARGET + "/cc/" + CC_VERSION;

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

/**
 * 目的：夹具里的"工具链"——两种都不可用，但**不可用的方式**不同，因此走不同分支：
 *   - 默认：一个不是可执行文件的 fake-cargo（SOLOMNI_CARGO 显式指定）⇒ 只告警，流程照旧；
 *   - borrowStub：把 node 复制成 PATH 上的 cargo ⇒ 第一道探测（--version）过得去，构建探测不成立 ⇒ 不借用。
 *   （node 打印版本、-vV 给不出 host，正好代表"能跑但构建不了"的借用工具链。）
 */
function fixtureEnv(dir, c) {
  // projectStub 要的就是"项目内那套"这条路：不能被 SOLOMNI_CARGO 抢走。
  if (c.projectStub) {
    const env = { ...process.env };
    delete env.SOLOMNI_CARGO;
    return env;
  }
  if (c.borrowStub) {
    const bin = path.join(dir, "bin");
    fs.mkdirSync(bin, { recursive: true });
    const stub = path.join(bin, IS_WIN ? "cargo.exe" : "cargo");
    try { fs.linkSync(process.execPath, stub); } catch (e) { fs.copyFileSync(process.execPath, stub); }
    const key = Object.keys(process.env).find((k) => k.toUpperCase() === "PATH") || "PATH";
    const env = { ...process.env, [key]: bin + path.delimiter + (process.env[key] || "") };
    delete env.SOLOMNI_CARGO;
    return env;
  }
  // 假 cargo：env.js 只查它在不在（也就不会去装 Rust、不碰网络）。
  fs.writeFileSync(path.join(dir, "fake-cargo"), "");
  return { ...process.env, SOLOMNI_CARGO: path.join(dir, "fake-cargo") };
}

async function runCase(c) {
  const dir = path.join(SCRATCH, c.name);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, ".tools"), { recursive: true });
  fs.copyFileSync(path.join(ROOT, "env.js"), path.join(dir, "env.js"));
  if (c.projectStub) {
    // 项目内工具链存在（下载中断留下的半套也长这样）：路径对、跑得起来，但构建不出东西。
    const bin = path.join(dir, "platform", IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux", "cargo", "bin");
    fs.mkdirSync(bin, { recursive: true });
    const stub = path.join(bin, IS_WIN ? "cargo.exe" : "cargo");
    try { fs.linkSync(process.execPath, stub); } catch (e) { fs.copyFileSync(process.execPath, stub); }
  }
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
    const env = fixtureEnv(dir, c);
    // 调用方环境里的代理不能连夹具的 loopback 应答源一起代理（代理回 502 会让下载用例假失败）。
    for (const k of ["NO_PROXY", "no_proxy"]) env[k] = "127.0.0.1,localhost";
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
      // 显式指定（SOLOMNI_CARGO）探测不过时只告警，不推翻用户的选择、也不改流程。
      assert.match(out, /SOLOMNI_CARGO 指定的工具链构建不出二进制/, "显式指定的工具链要如实告警");
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
    // 本机踩过的坑：借来的 cargo 能打印版本，但没有链接器（Windows 上 msvc host 缺 VS C++）。
    // 这种工具链不算可用：不许宣告就绪，要报出借用不成立与原因。
    name: "borrowed-toolchain-that-cannot-build-is-not-used",
    borrowStub: true,
    mode: "range",
    check: ({ out, code }) => {
      assert.equal(code, 1, "借用不成立时不能宣告就绪：" + tail(out));
      assert.match(out, /借用不成立/, "要说明这套工具链构建不出二进制：" + tail(out));
      assert.ok(/host/.test(out), "要点名借来的 host");
      assert.ok(!/environment ready/.test(out), "不许打印 environment ready");
    },
  },
  {
    // 下载中断会留下"装了一半"的项目内工具链：它存在、能跑，但构建不了。这种状态不许被当成就绪。
    name: "half-installed-project-toolchain-is-reported",
    projectStub: true,
    mode: "range",
    check: ({ out, code }) => {
      assert.equal(code, 1, "半套的项目内工具链不能宣告就绪：" + tail(out));
      assert.match(out, /项目内工具链构建不出二进制/, "要说明项目内工具链构建不了：" + tail(out));
      assert.match(out, /rustup toolchain uninstall/, "修复步骤要先卸掉半套（再装一次会回 unchanged）");
      assert.match(out, /node env.js setup/, "并指回安装入口");
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


// ---------- POSIX：宿主 cc 缺失时项目内 Zig 自举（本地夹具，不连外网） ----------

function findInPath(name, pathValue) {
  return (pathValue || process.env.PATH || "")
    .split(path.delimiter)
    .map((d) => d.trim().replace(/^"|"$/g, ""))
    .filter(Boolean)
    .map((d) => path.join(d, name))
    .find((p) => { try { return fs.existsSync(p); } catch (e) { return false; } }) || null;
}

/** 目的：造一份最小 Zig 归档（未压缩 tar；env.js 的 tar -xf 会自动识别）。假 zig 只回显参数。 */
function makeTarBody(dir) {
  const src = path.join(dir, "tar-src");
  const pkg = path.join(src, "zig-demo");
  fs.mkdirSync(pkg, { recursive: true });
  fs.writeFileSync(
    path.join(pkg, "zig"),
    "#!/bin/sh\nprintf 'GLOBAL=%s\\nLOCAL=%s\\n' \"$ZIG_GLOBAL_CACHE_DIR\" \"$ZIG_LOCAL_CACHE_DIR\"\nprintf '%s\\n' \"$@\"\n",
    { mode: 0o755 },
  );
  const tar = path.join(dir, "zig-fixture.tar");
  const r = spawnSync("tar", ["-cf", tar, "-C", src, "zig-demo"], { encoding: "utf8" });
  assert.equal(r.status, 0, "造 tar 夹具失败：" + (r.stderr || ""));
  const body = fs.readFileSync(tar);
  return { body, sha: crypto.createHash("sha256").update(body).digest("hex") };
}

function startBodyServer(body) {
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push({ url: req.url, range: req.headers.range || null });
    res.writeHead(200, { "Content-Length": body.length });
    res.end(body);
  });
  return new Promise((done) => {
    server.listen(0, "127.0.0.1", () => {
      done({ server, requests, base: "http://127.0.0.1:" + server.address().port });
    });
  });
}

const POSIX_OS = IS_MAC ? "macos" : "linux";

/** 目的：夹具项目根——copied env.js、当前平台清单条目、假 cargo / rustc、只含 curl/tar 的 PATH。 */
function writePosixFixture(dir, opts) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
  fs.copyFileSync(path.join(ROOT, "env.js"), path.join(dir, "env.js"));
  for (const name of ["curl", "tar"]) {
    const found = findInPath(name);
    assert.ok(found, "夹具需要系统 " + name);
    fs.symlinkSync(found, path.join(dir, "bin", name));
  }
  if (opts.hostCC) {
    fs.writeFileSync(path.join(dir, "bin", "cc"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  }
  const cargoBin = path.join(dir, "platform", POSIX_OS, "cargo", "bin");
  fs.mkdirSync(cargoBin, { recursive: true });
  fs.writeFileSync(
    path.join(cargoBin, "cargo"),
    "#!/bin/sh\nif [ \"$1\" = \"-vV\" ]; then printf 'cargo 1.99.1\\nhost: " + HOST + "\\n'; exit 0; fi\nexit 0\n",
    { mode: 0o755 },
  );
  // 宿主或项目内 cc 可用时探测成功；否则报真机上的缺链接器错误。
  fs.writeFileSync(
    path.join(cargoBin, "rustc"),
    "#!/bin/sh\nif command -v cc >/dev/null 2>&1; then exit 0; fi\necho 'error: linker `cc` not found' >&2\nexit 1\n",
    { mode: 0o755 },
  );
  fs.writeFileSync(
    path.join(dir, "dependencies.yaml"),
    "deps:\n" +
      "  - id: toolchain.cc." + TARGET + "\n" +
      "    install: " + CC_INSTALL + "\n" +
      "    probe: " + CC_INSTALL + "/bin/gcc\n" +
      "    source: fixture.zig\n" +
      "    toolchain:\n" +
      "      tool: cc\n" +
      "      version: " + CC_VERSION + "\n" +
      "      target: " + TARGET + "\n" +
      "      bins: [bin]\n",
  );
  fs.writeFileSync(
    path.join(dir, "links.yaml"),
    "fixture.zig: " + opts.url + "\nfixture.zig.sha256: " + opts.sha + "\n",
  );
}

function baseEnv(dir, override) {
  const key = Object.keys(process.env).find((k) => k.toUpperCase() === "PATH") || "PATH";
  const env = { ...process.env, [key]: path.join(dir, "bin") };
  if (override) env.SOLOMNI_CARGO = path.join(dir, "platform", POSIX_OS, "cargo", "bin", "cargo");
  else delete env.SOLOMNI_CARGO;
  for (const k of ["NO_PROXY", "no_proxy"]) env[k] = "127.0.0.1,localhost";
  delete env.SOLOMNI_MINGW_BIN;
  delete env.SOLOMNI_ACCEPT_ASSET_CHANGE;
  return env;
}

function runEnv(dir, env, args) {
  return new Promise((resolve, reject) => {
    const logPath = path.join(dir, "run.log");
    const fd = fs.openSync(logPath, "w");
    const child = spawn(process.execPath, ["env.js"].concat(args), {
      cwd: dir, env, stdio: ["ignore", fd, fd],
    });
    const timer = setTimeout(() => {
      try { child.kill(); } catch (e) { /* 已退出 */ }
      reject(new Error("env.js 超时（60s）"));
    }, 60000);
    child.on("error", (e) => { clearTimeout(timer); fs.closeSync(fd); reject(e); });
    child.on("exit", (code) => {
      clearTimeout(timer);
      fs.closeSync(fd);
      resolve({ code, out: fs.readFileSync(logPath, "utf8") });
    });
  });
}

/** 目的：POSIX 上逐条钉住“宿主 cc 可用就不下载 / 缺失才自举 / 不擅自下载 / 校验闸 / macOS 前置”。 */
async function runPosixCases() {
  fs.rmSync(SCRATCH, { recursive: true, force: true });
  const tailOut = (out) => "\n" + out.split(/\r?\n/).slice(-12).join("\n");

  if (!IS_MAC) {
    // ① 宿主没有 cc：--yes 后自举 Zig，包装可用，--target= 翻译正确。
    {
      const name = "posix-bootstraps-zig-when-host-cc-missing";
      const dir = path.join(SCRATCH, name);
      fs.mkdirSync(dir, { recursive: true });
      const { body, sha } = makeTarBody(dir);
      const { server, requests, base } = await startBodyServer(body);
      try {
        writePosixFixture(dir, { url: base + "/zig.tar", sha });
        const r = await runEnv(dir, baseEnv(dir, true), ["setup", "--yes"]);
        assert.equal(r.code, 0, "自举要成功：" + tailOut(r.out));
        assert.match(r.out, /downloading/, "要下载 Zig");
        assert.match(r.out, /sha256 verified/, "要过完整性闸");
        assert.match(r.out, /project-local C toolchain ready/, "要装好项目内工具链");
        assert.match(r.out, /environment ready/, "构建探测要通过");
        const bin = path.join(dir, CC_INSTALL, "bin");
        for (const n of ["zig", "cc", "gcc", "g++", "c++", "clang", "clang++"]) {
          assert.ok(fs.existsSync(path.join(bin, n)), "缺包装：" + n);
        }
        const w = spawnSync(path.join(bin, "cc"), ["--target=x86_64-unknown-linux-gnu"], {
          env: { ...process.env, PATH: "/usr/bin:/bin" }, encoding: "utf8",
        });
        assert.equal(w.status, 0, "包装要能执行：" + (w.stderr || ""));
        assert.match(w.stdout, /--target=x86_64-linux-gnu/, "要把 Rust 三元组翻成 Zig 认的形态：" + w.stdout);
        assert.ok(!/unknown-/.test(w.stdout), "不能把 unknown- 传给 Zig：" + w.stdout);
        const cacheBase = path.join(dir, CC_INSTALL, "bin") + "/../cache";
        assert.ok(w.stdout.includes("GLOBAL=" + cacheBase + "/global"), "Zig 全局缓存要钉在安装目录内：" + w.stdout);
        assert.ok(w.stdout.includes("LOCAL=" + cacheBase + "/local"), "Zig 本地缓存要钉在安装目录内：" + w.stdout);
        assert.ok(!fs.existsSync(path.join(dir, ".tools", "zig.tar.xz")), "装完要删归档");
        const active = JSON.parse(fs.readFileSync(path.join(dir, ".tools", "toolchains", "active.json"), "utf8"));
        assert.equal(active.target, TARGET, "active.json 要写当前目标");
        assert.ok(active.profile.includes("cc"), "profile 要包含 cc：" + JSON.stringify(active.profile));
        const cc = active.tools.find((t) => t && t.id === "cc");
        assert.ok(cc, "tools 要登记 cc：" + JSON.stringify(active.tools));
        assert.equal(cc.version, CC_VERSION, "要登记清单版本");
        assert.deepEqual(cc.bin, [CC_INSTALL + "/bin"], "bin 要指向安装目录下的 bin");
        assert.ok(requests.length > 0, "夹具源要被请求");
      } finally {
        server.close();
      }
      console.log("PASS " + name);
    }

    // ② 无 TTY 且没给 --yes：不擅自下载，停点明确指向 C 工具链（不是重装 Rust）。
    {
      const name = "posix-no-tty-does-not-download-and-points-at-c-toolchain";
      const dir = path.join(SCRATCH, name);
      fs.mkdirSync(dir, { recursive: true });
      const { body, sha } = makeTarBody(dir);
      const { server, requests, base } = await startBodyServer(body);
      try {
        writePosixFixture(dir, { url: base + "/zig.tar", sha });
        const r = await runEnv(dir, baseEnv(dir, false), ["setup"]);
        assert.equal(r.code, 1, "无 TTY 要如实失败：" + tailOut(r.out));
        assert.match(r.out, /无交互终端/, "要说明没有擅自下载");
        assert.match(r.out, /缺少可用的 C 链接器/, "要指向 C 工具链：" + tailOut(r.out));
        assert.match(r.out, /node env\.js setup --yes/, "要给项目内自举入口");
        assert.equal(requests.length, 0, "没同意就不该请求下载");
        assert.ok(!fs.existsSync(path.join(dir, CC_INSTALL)), "没同意就不该装");
      } finally {
        server.close();
      }
      console.log("PASS " + name);
    }

    // ③ 半个项目内 cc（只有 gcc、没有 rustc 要用的 cc）：必须重装，不能当成就绪。
    {
      const name = "posix-half-installed-zig-is-reinstalled";
      const dir = path.join(SCRATCH, name);
      fs.mkdirSync(dir, { recursive: true });
      const { body, sha } = makeTarBody(dir);
      const { server, base } = await startBodyServer(body);
      try {
        writePosixFixture(dir, { url: base + "/zig.tar", sha });
        const half = path.join(dir, CC_INSTALL, "bin");
        fs.mkdirSync(half, { recursive: true });
        fs.writeFileSync(path.join(half, "gcc"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
        // 已有别的工具登记：重装 cc 时必须保留，不能整份覆盖。
        const activePath = path.join(dir, ".tools", "toolchains", "active.json");
        fs.writeFileSync(activePath, JSON.stringify({
          target: TARGET,
          profile: ["python", "cc"],
          tools: [{ id: "python", version: "9.9.9", bin: [".tools/python/bin"], env: {} }],
        }, null, 2) + "\n");
        const r = await runEnv(dir, baseEnv(dir, true), ["setup", "--yes"]);
        assert.equal(r.code, 0, "半个安装要能被重装修好：" + tailOut(r.out));
        assert.match(r.out, /downloading/, "缺 cc 包装必须重新安装，不能当成就绪");
        assert.ok(fs.existsSync(path.join(half, "cc")), "重装后要有 cc 包装");
        const active = JSON.parse(fs.readFileSync(activePath, "utf8"));
        assert.ok(active.tools.some((t) => t && t.id === "python"), "已有 python 条目要保留");
        assert.ok(active.tools.some((t) => t && t.id === "cc"), "重装后要登记 cc");
      } finally {
        server.close();
      }
      console.log("PASS " + name);
    }

    // ④ sha256 闸：不符时拒绝解包，保留归档与两值。
    {
      const name = "posix-sha-mismatch-keeps-archive-and-refuses";
      const dir = path.join(SCRATCH, name);
      fs.mkdirSync(dir, { recursive: true });
      const { body } = makeTarBody(dir);
      const { server, base } = await startBodyServer(body);
      try {
        writePosixFixture(dir, { url: base + "/zig.tar", sha: "0".repeat(64) });
        const r = await runEnv(dir, baseEnv(dir, true), ["setup", "--yes"]);
        assert.equal(r.code, 1, "哈希不符要拒绝：" + tailOut(r.out));
        assert.match(r.out, /sha256 不符/, "要报哈希不符");
        assert.ok(fs.existsSync(path.join(dir, ".tools", "zig.tar.xz")), "不许删归档");
        assert.ok(!fs.existsSync(path.join(dir, CC_INSTALL)), "不符的归档不准解包");
      } finally {
        server.close();
      }
      console.log("PASS " + name);
    }
  }

  // ⑤ 宿主 cc 能构建：零下载，直接用宿主工具链。
  {
    const name = "posix-no-download-when-host-cc-builds";
    const dir = path.join(SCRATCH, name);
    fs.mkdirSync(dir, { recursive: true });
    const { body, sha } = makeTarBody(dir);
    const { server, requests, base } = await startBodyServer(body);
    try {
      writePosixFixture(dir, { url: base + "/zig.tar", sha, hostCC: true });
      const r = await runEnv(dir, baseEnv(dir, true), ["setup", "--yes"]);
      assert.equal(r.code, 0, "宿主能构建要直接通过：" + tailOut(r.out));
      assert.ok(!/downloading/.test(r.out), "宿主 cc 可用时不该下载");
      assert.equal(requests.length, 0, "宿主 cc 可用时不该请求夹具源");
      assert.ok(!fs.existsSync(path.join(dir, ".tools", "toolchains", TARGET, "cc")), "不该装项目内 cc");
    } finally {
      server.close();
    }
    console.log("PASS " + name);
  }

  // ⑥ macOS 缺 CLT：Zig 不含 SDK，直接给准确前置，不下载。
  if (IS_MAC) {
    const name = "macos-missing-clt-reports-prerequisite";
    const dir = path.join(SCRATCH, name);
    fs.mkdirSync(dir, { recursive: true });
    const { body, sha } = makeTarBody(dir);
    const { server, requests, base } = await startBodyServer(body);
    try {
      writePosixFixture(dir, { url: base + "/zig.tar", sha });
      const r = await runEnv(dir, baseEnv(dir, true), ["setup", "--yes"]);
      assert.equal(r.code, 1, "缺 CLT 要如实失败：" + tailOut(r.out));
      assert.match(r.out, /Xcode Command Line Tools/, "要说明 Apple SDK 前置");
      assert.match(r.out, /xcode-select --install/, "要给安装命令");
      assert.equal(requests.length, 0, "不该假装 Zig 能替代 SDK");
    } finally {
      server.close();
    }
    console.log("PASS " + name);
  }

  console.log("POSIX-ENV-BOOTSTRAP-OK");
}

/**
 * 目的：安装身份跨语言钉住——同一份合成清单，env.js 与 Rust install_identity 必须算出同一指纹。
 * 为什么机器查：env.js 装完按身份写产品台账，产品按它判「已装」；材料布局漂了就是重复下载。
 * 约束：常量与 `src/tests/provision/service.rs` 的 install_identity_is_pinned_across_languages 共享。
 */
function toolchainIdentityPin() {
  const envLayer = require(path.join(ROOT, "env.js"));
  const dep = {
    id: "toolchain.python.pin-test", tool: "python", version: "9.9.9", target: "test-target",
    source: "test.source", sha256: "ab".repeat(32), sizeMb: 48, wrappers: false,
    installRel: "test/install", probeRel: "test/install/probe", binsJoined: "bin,sbin",
  };
  assert.equal(
    envLayer.installIdentity(dep),
    "99076528b675dd67741122f3fa801f6c8ca87f9359bb883441b13d657537eb26",
    "身份材料布局要与 Rust install_identity 逐字节一致"
  );
  console.log("PASS toolchain-identity-pin");
}

/**
 * 目的：python/node 脚本侧安装链自检——真清单布局 + 本地应答源，钉住同意闸、校验闸、探针闸、
 *   台账身份与 active.json 合并登记、在场跳过（不重复下载）。
 * 约束：夹具根只写 target/test-scratch/；假 cargo 走 override、binutils 预置 / 宿主 cc 桩绕过 Rust/C 引导，
 *   不触外网、不装 Rust。
 */
async function toolchainInstallCases() {
  const envLayer = require(path.join(ROOT, "env.js"));
  const dir = path.join(SCRATCH, "toolchain-install");
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(path.join(ROOT, "env.js"), path.join(dir, "env.js"));
  fs.copyFileSync(path.join(ROOT, "dependencies.yaml"), path.join(dir, "dependencies.yaml"));
  const pyDep = envLayer.readToolchainDep("python");
  const nodeDep = envLayer.readToolchainDep("node");
  assert.ok(pyDep && nodeDep, "清单要有当前平台的 python/node 条目");
  // 清单函数给的是相对字段；在场判定一律落在夹具根，不碰真仓库的安装。
  const pyProbe = path.join(dir, pyDep.probeRel);
  const nodeProbe = path.join(dir, nodeDep.probeRel);
  const pyInstall = path.join(dir, pyDep.installRel);

  // 按真清单布局造夹具归档：bin 目录与探针文件都从清单条目派生，不在测试里抄第二份布局。
  const makeArchive = (dep, withProbe) => {
    const s = path.join(dir, "stage-" + dep.tool + (withProbe ? "" : "-nopro"));
    fs.rmSync(s, { recursive: true, force: true });
    for (const b of dep.bins) {
      fs.mkdirSync(path.join(s, ...b.slice(dep.installRel.length + 1).split("/")), { recursive: true });
    }
    if (withProbe) {
      const p = path.join(s, path.relative(dep.install, dep.probe));
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, "");
    }
    const isZip = IS_WIN && dep.tool === "node";
    const out = path.join(dir, dep.tool + (isZip ? ".zip" : ".tar.gz"));
    const tar = IS_WIN ? path.join(process.env.SystemRoot || "C:/Windows", "System32", "tar.exe") : "tar";
    const args = isZip ? ["-a", "-cf", out, "-C", s, "."] : ["-czf", out, "-C", s, "."];
    const r = spawnSync(tar, args, { encoding: "utf8" });
    assert.equal(r.status, 0, "造夹具归档失败：" + (r.stderr || r.error || ""));
    return { buf: fs.readFileSync(out), sha: crypto.createHash("sha256").update(fs.readFileSync(out)).digest("hex") };
  };
  const pyArc = makeArchive(pyDep, true);
  const nodeArc = makeArchive(nodeDep, true);
  const pyBad = makeArchive(pyDep, false);

  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push(req.url);
    const body = req.url.includes("python") ? (req.url.includes("bad") ? pyBad.buf : pyArc.buf)
      : req.url.includes("node") ? nodeArc.buf : null;
    if (!body) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { "Content-Length": body.length });
    res.end(body);
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  const base = "http://127.0.0.1:" + server.address().port;
  const pyUrl = base + "/python.tar.gz";
  const nodeUrl = base + (IS_WIN ? "/node.zip" : "/node.tar.gz");
  const rewrite = (text, key, val) => text.split(/\r?\n/).map((l) => (l.startsWith(key + ":") ? key + ": " + val : l)).join("\n");
  const writeLinks = (pySha, pySrc) => {
    let text = fs.readFileSync(path.join(ROOT, "links.yaml"), "utf8");
    text = rewrite(text, pyDep.source, pySrc || pyUrl);
    text = rewrite(text, pyDep.source + ".sha256", pySha);
    text = rewrite(text, nodeDep.source, nodeUrl);
    text = rewrite(text, nodeDep.source + ".sha256", nodeArc.sha);
    fs.writeFileSync(path.join(dir, "links.yaml"), text + "\n");
  };
  writeLinks(pyArc.sha);

  // 假 cargo（override）+ binutils 预置（Windows）/ 宿主 cc 桩（POSIX）：绕过 Rust/C 引导，只测工具链安装链。
  const fakeCargo = path.join(dir, "fake-cargo");
  fs.writeFileSync(fakeCargo, "");
  const mingwBin = path.join(dir, ".tools", "mingw64", "bin");
  fs.mkdirSync(mingwBin, { recursive: true });
  fs.writeFileSync(path.join(mingwBin, "dlltool.exe"), "");
  fs.writeFileSync(path.join(mingwBin, "as.exe"), "");
  const ambient = path.join(dir, "ambient-bin");
  fs.mkdirSync(ambient, { recursive: true });
  if (!IS_WIN) fs.writeFileSync(path.join(ambient, "cc"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  // 宿主 cc 桩前置（绕过 Zig 引导）；真 PATH 垫底——curl/tar 这些系统工具还要找得到。
  const env = { ...process.env, SOLOMNI_CARGO: fakeCargo, PATH: ambient + path.delimiter + (process.env.PATH || ""), NO_PROXY: "127.0.0.1,localhost", no_proxy: "127.0.0.1,localhost" };
  delete env.SOLOMNI_CARGO_HOME;
  delete env.SOLOMNI_MINGW_BIN;
  delete env.SOLOMNI_ACCEPT_ASSET_CHANGE;
  // 必须异步 spawn：应答源跑在本进程事件循环里，spawnSync 会把它堵死（curl 干等到超时）。
  const runSetup = (args) => new Promise((resolve, reject) => {
    const logPath = path.join(dir, "setup-run.log");
    const fd = fs.openSync(logPath, "w");
    const child = spawn(process.execPath, ["env.js", "setup"].concat(args), { cwd: dir, env, stdio: ["ignore", fd, fd] });
    const timer = setTimeout(() => { try { child.kill(); } catch {} reject(new Error("setup 超时（120s）")); }, 120000);
    child.on("error", (e) => { clearTimeout(timer); fs.closeSync(fd); reject(e); });
    child.on("exit", (code) => {
      clearTimeout(timer);
      fs.closeSync(fd);
      const out = fs.readFileSync(logPath, "utf8");
      resolve({ status: code, stdout: out, stderr: out });
    });
  });
  const reset = () => {
    fs.rmSync(path.join(dir, ".tools", "toolchains"), { recursive: true, force: true });
    fs.rmSync(path.join(dir, ".vm"), { recursive: true, force: true });
    for (const f of fs.readdirSync(path.join(dir, ".tools"))) {
      if (/^(python|node)\.(tar\.gz|zip|tar\.xz)$/.test(f)) fs.rmSync(path.join(dir, ".tools", f), { force: true });
    }
    requests.length = 0;
  };
  const readJson = (p) => JSON.parse(fs.readFileSync(p, "utf8"));

  try {
    // ① --yes 全装：探针在场、台账身份与指纹函数一致、active.json 登记 python/node、各请求一次。
    let r = await runSetup(["--yes"]);
    assert.equal(r.status, 0, "setup --yes 要成功：error=" + (r.error && r.error.message) + " signal=" + r.signal + "\n" + (r.stderr || "") + (r.stdout || ""));
    assert.ok(fs.existsSync(pyProbe), "python 探针要在场");
    assert.ok(fs.existsSync(nodeProbe), "node 探针要在场");
    const ledger = readJson(path.join(dir, ".vm", "state", "deps.json"));
    assert.equal(ledger[pyDep.id], envLayer.installIdentity({ ...pyDep, sha256: pyArc.sha }), "python 台账身份需要与指纹函数一致");
    assert.equal(ledger[nodeDep.id], envLayer.installIdentity({ ...nodeDep, sha256: nodeArc.sha }), "node 台账身份需要与指纹函数一致");
    const active = readJson(path.join(dir, ".tools", "toolchains", "active.json"));
    for (const dep of [pyDep, nodeDep]) {
      const t = active.tools.find((x) => x && x.id === dep.tool);
      assert.ok(t, "active.json 要登记 " + dep.tool);
      assert.equal(t.version, dep.version, dep.tool + " 版本按清单");
      assert.deepEqual(t.bin, dep.bins, dep.tool + " bin 目录按清单");
    }
    assert.equal(requests.filter((u) => u.includes("python")).length, 1, "python 只请求一次");
    assert.equal(requests.filter((u) => u.includes("node")).length, 1, "node 只请求一次");
    assert.ok(!fs.existsSync(path.join(dir, ".tools", "python.tar.gz")), "装完归档要删");
    console.log("PASS toolchain-install-full");

    // ② 在场跳过：再跑一次 --yes，零请求（产品 provision 装过的也不动）。
    requests.length = 0;
    r = await runSetup(["--yes"]);
    assert.equal(r.status, 0, "在场重跑要成功：" + (r.stderr || ""));
    assert.equal(requests.length, 0, "探针在场不许再下载");
    console.log("PASS toolchain-install-skip-present");

    // ③ 无 TTY 无 --yes：不下载、如实报、指路 setup --yes。
    reset();
    r = await runSetup([]);
    assert.equal(r.status, 0, "cargo 就绪时 setup 不因工具链缺项失败：" + (r.stderr || ""));
    assert.equal(requests.length, 0, "没同意不许请求源");
    assert.ok(!fs.existsSync(pyProbe) && !fs.existsSync(nodeProbe), "没同意不许装");
    assert.match(r.stderr, /无交互终端/, "要明说无交互终端不下载");
    assert.match(r.stderr, /setup --yes/, "要指路 setup --yes");
    console.log("PASS toolchain-install-no-tty");

    // ④ sha256 不符：退出 1、报两值与来源、保留归档、不装。
    reset();
    writeLinks("0".repeat(64));
    r = await runSetup(["--yes"]);
    assert.equal(r.status, 1, "哈希不符要如实失败");
    assert.match(r.stderr, /sha256 不符/, "要报哈希不符");
    assert.ok(r.stderr.includes("0".repeat(64)), "要报台账值");
    assert.ok(r.stderr.includes(pyArc.sha), "要报实际值");
    assert.ok(fs.existsSync(path.join(dir, ".tools", "python.tar.gz")), "不符的归档不许删");
    assert.ok(!fs.existsSync(pyProbe), "不符不准装");
    console.log("PASS toolchain-install-sha-mismatch");

    // ⑤ 归档缺探针（布局不符）：退出 1、不替换安装目录、临时目录清掉。
    reset();
    writeLinks(pyBad.sha, base + "/python-bad.tar.gz");
    r = await runSetup(["--yes"]);
    assert.equal(r.status, 1, "布局不符要如实失败");
    assert.match(r.stderr, /找不到探针/, "要报探针缺失");
    assert.ok(!fs.existsSync(pyInstall), "安装目录不许被半份替换");
    assert.ok(!fs.readdirSync(path.dirname(pyInstall)).some((f) => f.includes(".tmp-")), "临时目录要清掉");
    console.log("PASS toolchain-install-bad-layout");
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

/**
 * 目的：工具链注入自检——active.json 登记的项目内 python/node/cc bin 必须前置进 --print-env 的 PATH，
 *   判据与产品侧 compose 同一套（target 不符不注入、bin 只认相对普通组件且目录在场、env 收非空键值）。
 * 为什么机器查：门禁与启动器子进程的工具链解析全经这一份注入；判据漂了就是宿主依赖回潮。
 * 约束：夹具根只写 target/test-scratch/；假 cargo 让解析走 override，不触网、不装东西。
 */
function toolchainInjectionSelfCheck() {
  const dir = path.join(SCRATCH, "toolchain-injection");
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(path.join(ROOT, "env.js"), path.join(dir, "env.js"));
  fs.copyFileSync(path.join(ROOT, "links.yaml"), path.join(dir, "links.yaml"));
  const os = IS_WIN ? "windows" : IS_MAC ? "macos" : "linux";
  const arch = process.arch === "x64" ? "x86_64" : process.arch === "arm64" ? "aarch64" : process.arch;
  const target = os + "-" + arch;
  const rel = (...segs) => segs.join("/");
  const pyBinRel = rel(".tools", "toolchains", target, "python", "9.9.9", "pbin");
  const nodeBinRel = rel(".tools", "toolchains", target, "node", "9.9.9", "nbin");
  const pyBin = path.join(dir, ...pyBinRel.split("/"));
  const nodeBin = path.join(dir, ...nodeBinRel.split("/"));
  fs.mkdirSync(pyBin, { recursive: true });
  fs.mkdirSync(nodeBin, { recursive: true });
  const nodeExe = path.join(nodeBin, IS_WIN ? "node.exe" : "node");
  fs.writeFileSync(nodeExe, "");
  const ambient = path.join(dir, "ambient-bin");
  fs.mkdirSync(ambient, { recursive: true });
  const activeFile = path.join(dir, ".tools", "toolchains", "active.json");
  fs.mkdirSync(path.dirname(activeFile), { recursive: true });
  const writeActive = (doc) => fs.writeFileSync(activeFile, JSON.stringify(doc, null, 2));
  const goodDoc = {
    target,
    profile: ["python", "node", "cc"],
    tools: [
      { id: "python", version: "9.9.9", bin: [pyBinRel], env: { E2E_TC_VAR: "1", EMPTY: "" } },
      { id: "node", version: "9.9.9", bin: [nodeBinRel], env: {} },
      { id: "evil", version: "1", bin: ["../escape", path.join(dir, "abs"), rel(".tools", "gone")], env: {} },
    ],
  };
  // 假 cargo（空文件）：解析走 SOLOMNI_CARGO override，构建探测必然失败但 --print-env 照常输出。
  const fakeCargo = path.join(dir, "fake-cargo");
  fs.writeFileSync(fakeCargo, "");
  const env = { ...process.env, SOLOMNI_CARGO: fakeCargo, PATH: ambient };
  delete env.SOLOMNI_CARGO_HOME;
  delete env.SOLOMNI_MINGW_BIN;
  const printEnv = () => {
    const r = spawnSync(process.execPath, ["env.js", "--print-env"], { cwd: dir, env, encoding: "utf8", timeout: 60000 });
    assert.equal(r.status, 0, "--print-env 要正常退出：" + (r.stderr || ""));
    return JSON.parse(r.stdout);
  };
  const pathParts = (j) => j.env.PATH.split(path.delimiter);

  // ① 正常登记：python/node bin 前置（在宿主垫底之前），nodeBin 指向项目内可执行，env 收非空键值。
  writeActive(goodDoc);
  let j = printEnv();
  let parts = pathParts(j);
  assert.ok(parts.includes(pyBin), "python bin 要进 PATH：" + j.env.PATH);
  assert.ok(parts.includes(nodeBin), "node bin 要进 PATH：" + j.env.PATH);
  assert.ok(parts.indexOf(pyBin) < parts.indexOf(ambient), "项目内 bin 要在宿主 PATH 之前");
  assert.ok(parts.indexOf(nodeBin) < parts.indexOf(ambient), "项目内 bin 要在宿主 PATH 之前");
  assert.equal(j.nodeBin, nodeExe, "nodeBin 要指向项目内 node 可执行");
  assert.equal(j.env.E2E_TC_VAR, "1", "工具 env 的非空键值要导出");
  assert.ok(!("EMPTY" in j.env) || j.env.EMPTY !== "", "空值键不导出");
  assert.deepEqual(j.toolchains.map((t) => t.id).sort(), ["node", "python"], "在场工具如实列出");
  assert.deepEqual(j.toolchainMissing, ["cc"], "profile 声明但没装的工具要报缺");
  // 不安全 bin（相对逃逸 / 绝对路径 / 不在场）一律不进 PATH。
  assert.ok(!parts.some((p) => p.includes("escape")), "相对逃逸的 bin 不许进 PATH");
  assert.ok(!parts.includes(path.join(dir, "abs")), "绝对路径的 bin 不许进 PATH");
  assert.ok(!parts.some((p) => p.endsWith("gone")), "不在场的 bin 目录不许进 PATH");

  // ② target 不符：整份不注入（与产品侧同口径），nodeBin 给 null。
  writeActive({ ...goodDoc, target: "other-target" });
  j = printEnv();
  parts = pathParts(j);
  assert.ok(!parts.includes(pyBin) && !parts.includes(nodeBin), "target 不符不许注入");
  assert.equal(j.nodeBin, null, "target 不符 nodeBin 要给 null");
  assert.deepEqual(j.toolchains, [], "target 不符工具表为空");

  // ③ active.json 不在场：回退宿主 PATH，不报错。
  fs.rmSync(activeFile);
  j = printEnv();
  assert.equal(j.nodeBin, null, "未登记 nodeBin 要给 null");
  assert.ok(pathParts(j).includes(ambient), "未登记宿主 PATH 照常垫底");

  // ④ 状态入口也要报工具链事实（登记 / 未登记两种形态）。
  writeActive(goodDoc);
  let st = spawnSync(process.execPath, ["env.js"], { cwd: dir, env, encoding: "utf8", timeout: 60000 });
  assert.equal(st.status, 0, "状态要正常退出：" + (st.stderr || ""));
  assert.match(st.stdout, /^toolchains: 项目内/m, "状态要报项目内工具链");
  assert.match(st.stdout, /^project node: /m, "状态要报项目内 node");
  fs.rmSync(activeFile);
  st = spawnSync(process.execPath, ["env.js"], { cwd: dir, env, encoding: "utf8", timeout: 60000 });
  assert.match(st.stdout, /^toolchains: 未登记/m, "未登记要如实报并指路 setup");

  fs.rmSync(dir, { recursive: true, force: true });
  console.log("PASS toolchain-injection");
}

/**
 * 目的：入口 re-exec 自检——项目内 node 在场时入口必须改由它跑（下游 process.execPath 统一）；
 *   守卫防循环、未登记回退宿主、项目内 node 起不来也回退宿主。
 * 约束：夹具 node 是宿主 node 的硬链接（跨卷退回复制）；只写 target/test-scratch/。
 */
function reexecSelfCheck() {
  const dir = path.join(SCRATCH, "reexec");
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(path.join(ROOT, "env.js"), path.join(dir, "env.js"));
  fs.copyFileSync(path.join(ROOT, "links.yaml"), path.join(dir, "links.yaml"));
  const os = IS_WIN ? "windows" : IS_MAC ? "macos" : "linux";
  const arch = process.arch === "x64" ? "x86_64" : process.arch === "arm64" ? "aarch64" : process.arch;
  const binRel = ".tools/toolchains/" + os + "-" + arch + "/node/9.9.9/nbin";
  const binDir = path.join(dir, ...binRel.split("/"));
  fs.mkdirSync(binDir, { recursive: true });
  const nodeExe = path.join(binDir, IS_WIN ? "node.exe" : "node");
  try { fs.linkSync(process.execPath, nodeExe); } catch (e) { fs.copyFileSync(process.execPath, nodeExe); }
  if (!IS_WIN) fs.chmodSync(nodeExe, 0o755);
  const activeFile = path.join(dir, ".tools", "toolchains", "active.json");
  const writeActive = () => fs.writeFileSync(activeFile, JSON.stringify({
    target: os + "-" + arch, profile: ["node"],
    tools: [{ id: "node", version: "9.9.9", bin: [binRel], env: {} }],
  }));
  fs.writeFileSync(path.join(dir, "entry.cjs"), 'require("./env.js").reexecProjectNode();\nconsole.log(process.execPath);\n');
  const runEntry = (extraEnv) => spawnSync(process.execPath, [path.join(dir, "entry.cjs")], {
    cwd: dir, env: { ...process.env, ...(extraEnv || {}) }, encoding: "utf8", timeout: 60000,
  });

  // ① 项目内 node 在场：入口改由它跑。
  writeActive();
  let r = runEntry();
  assert.equal(r.status, 0, "re-exec 要正常退出：" + (r.stderr || ""));
  assert.equal(r.stdout.trim(), nodeExe, "入口要改由项目内 node 跑");

  // ② 守卫：已 re-exec 过的进程不再 re-exec（防循环）。
  r = runEntry({ SOLOMNI_NODE_REEXEC: "1" });
  assert.equal(r.status, 0, "守卫路径要正常退出：" + (r.stderr || ""));
  assert.equal(r.stdout.trim(), process.execPath, "守卫在场不许再 re-exec");

  // ③ 未登记：照宿主 node 跑。
  fs.rmSync(activeFile);
  r = runEntry();
  assert.equal(r.status, 0, "未登记要正常退出：" + (r.stderr || ""));
  assert.equal(r.stdout.trim(), process.execPath, "未登记照宿主 node 跑");

  // ④ 项目内 node 起不来（空文件冒充）：如实报一句，照宿主继续。
  writeActive();
  fs.writeFileSync(nodeExe, "");
  r = runEntry();
  assert.equal(r.status, 0, "起不来要回退宿主而不是失败：" + (r.stderr || ""));
  assert.equal(r.stdout.trim(), process.execPath, "起不来照宿主 node 跑");
  assert.match(r.stderr, /项目内 node 起不来/, "回退要如实报原因");

  fs.rmSync(dir, { recursive: true, force: true });
  console.log("PASS entry-reexec");
}

/**
 * 目的：`node env.js`（只读状态）必须能自诊断——有 host、有构建探测结论；借用不可用时明说"不借用"。
 * 为什么机器查：本机掉进去的正是这条假绿——状态的每一行都"正常"，构建却必然失败（缺链接器）。
 */
function statusSelfCheck() {
  const st = spawnSync(process.execPath, ["env.js"], { cwd: ROOT, encoding: "utf8", timeout: 180000 });
  assert.equal(st.status, 0, "node env.js 要能正常退出：" + (st.stderr || ""));
  const out = st.stdout || "";
  assert.match(out, /^build: (可用|不可用)/m, "状态要给构建探测结论：" + tail(out));
  const resolved = /^resolved: .+\[([a-z]+)\]/m.exec(out);
  if (!resolved) {
    assert.match(out, /^build: 不可用/m, "没有 cargo 时构建结论必须是不可用");
    return;
  }
  assert.match(out, /^host: /m, "有工具链时状态要报 host：" + tail(out));
  if (resolved[1] === "ambient") {
    assert.equal(
      /^borrow: 不借用/m.test(out),
      /^build: 不可用/m.test(out),
      "借用不可用时状态必须明说不借用：" + tail(out),
    );
  }
}

if (!IS_WIN) {
  // Windows 的 MinGW 用例不适用；POSIX 走 Zig cc 自举用例（本地夹具，不连外网）。
  await runPosixCases();
  toolchainIdentityPin();
  await toolchainInstallCases();
  toolchainInjectionSelfCheck();
  reexecSelfCheck();
  statusSelfCheck();
  console.log("ENV-BOOTSTRAP-OK");
} else {
  fs.rmSync(SCRATCH, { recursive: true, force: true });
  for (const c of CASES) await runCase(c);
  toolchainIdentityPin();
  await toolchainInstallCases();
  toolchainInjectionSelfCheck();
  reexecSelfCheck();
  statusSelfCheck();
  console.log("ENV-BOOTSTRAP-OK");
}
