#!/usr/bin/env node
/**
 * Solomni launcher: platform check -> toolchain/build readiness -> run.
 * Default CLI; -webUI starts the Web UI. No business logic here.
 * Cross-platform: node start.js [-webUI] [--release] [--root <dir>] [--web-port <port>]
 * Rust missing? Ask, then install via rustup (GNU toolchain on Windows: no MSVC needed).
 * Windows GNU gap (rust-lang/rust#140704): windows-sys needs a WORKING dlltool.exe,
 * which means a complete binutils tree (dlltool.exe AND as.exe - the assembler it
 * shells out to). Preflight probes FIXED paths only (no disk scan); if no complete
 * tree exists, ask consent, then install portable winlibs MinGW into .tools/mingw64.
 * Download speed-tests our Release against upstream (optional SOLOMNI_GH_MIRROR
 * prefix proxy), verifies the archive, and resumes interrupted downloads.
 */
"use strict";
const { spawnSync, spawn } = require("child_process");
const readline = require("readline");
const path = require("path");
const fs = require("fs");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const RELEASE = process.argv.includes("--release");
const PROFILE = RELEASE ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const BUNDLED_CARGO = path.join(ROOT, "platform", "linux", "cargo", "bin", IS_WIN ? "cargo.exe" : "cargo");
const TOOLS = path.join(ROOT, ".tools");
const WINLIBS_BIN = path.join(TOOLS, "mingw64", "bin");
// Project-local rustup homes (same layout as the bundled linux toolchain):
const P_RUSTUP = path.join(ROOT, "platform", "windows", "rustup");
const P_CARGO = path.join(ROOT, "platform", "windows", "cargo");
// Release asset base for third-party redistributions (winlibs zip).
// Placeholder repo: fill in once the release is published.
const REL_BASE = "https://github.com/theRMM714/Solomni/releases/download/dependencies/";
// SHA256 of winlibs.zip. Empty = print hash on first successful download so you can
// pin it here; once pinned, a mismatch kills the run (protects mirror downloads).
const WINLIBS_SHA256 = "c1f52294597c0b73786b2a78eb5d176d89226d2f21875eab75e783a8b1cefcc4";

const log = (m) => console.log("[start] " + m);
const die = (m) => { console.error("[start] " + m); process.exit(1); };
let EXTRA_PATH = []; // dlltool location found by preflight; consumed by cargoEnv.

function binLocked(p) {
  // Windows refuses to open a running executable for writing (sharing violation),
  // so this probes whether another Solomni instance still holds the binary.
  // POSIX has no such lock, so it simply reports false there.
  if (!fs.existsSync(p)) return false;
  try {
    const fd = fs.openSync(p, "r+");
    fs.closeSync(fd);
    return false;
  } catch (e) {
    return true;
  }
}

function envPath(env) {
  // Windows env keys are case-insensitive but Node keeps them verbatim: the system
  // key is usually "Path". Writing env.PATH alongside it creates a DUPLICATE key,
  // and children then look up tools (as.exe) against a mangled PATH. Always read
  // and write through the existing key.
  const key = Object.keys(env).find((k) => k.toUpperCase() === "PATH");
  return key ? env[key] : "";
}

function setEnvPath(env, value) {
  const key = Object.keys(env).find((k) => k.toUpperCase() === "PATH") || "PATH";
  env[key] = value;
}

function locateDlltool(cargo) {
  // Fixed-path candidates only - never a disk scan. Priority order:
  // 1. project-local winlibs (.tools/mingw64) - ours, complete binutils
  // 2. conventional MSYS2 install dirs
  // 3. toolchain dirs (rust-mingw ships dlltool without its assembler)
  // 4. any directory that "where dlltool" already reports
  // Each candidate must be a COMPLETE binutils tree (binutilsReady: dlltool + as).
  // Returns { dir, broken }: dir is the first usable directory (or null), broken
  // lists directories that have dlltool.exe but no assembler.
  const dirs = [];
  const add = (p) => { if (p && dirs.indexOf(p) < 0) dirs.push(p); };
  add(WINLIBS_BIN);
  add("C:/msys64/mingw64/bin");
  add("C:/msys2/mingw64/bin");
  const rustc = path.join(path.dirname(cargo), IS_WIN ? "rustc.exe" : "rustc");
  const v = spawnSync(rustc, ["--print", "sysroot"], { encoding: "utf8" });
  const sysroot = (v.stdout || "").trim();
  if (sysroot && fs.existsSync(sysroot)) {
    const gnu = path.join(sysroot, "lib", "rustlib", "x86_64-pc-windows-gnu", "bin");
    add(path.join(gnu, "self-contained"));
    add(path.join(gnu, "gdb.debug"));
    add(gnu);
  }
  const w = spawnSync("where", ["dlltool"], { encoding: "utf8" });
  for (const line of (w.stdout || "").split(/\r?\n/)) {
    const t = line.trim();
    if (t && fs.existsSync(t)) add(path.dirname(t));
  }
  const broken = [];
  for (const d of dirs) {
    if (!fs.existsSync(path.join(d, "dlltool.exe"))) continue;
    if (binutilsReady(d)) return { dir: d, broken };
    broken.push(d);
  }
  return { dir: null, broken };
}

function binutilsReady(dir) {
  // dlltool shells out to the GNU assembler when building an import library, so a
  // usable directory needs BOTH dlltool.exe and as.exe. Static check by design:
  // trial-running dlltool proved unreliable (it reported a working winlibs install
  // as broken on Windows), and this pair is exactly what distinguishes the
  // rust-mingw copy (dlltool only) from a complete binutils tree.
  if (!dir) return false;
  return fs.existsSync(path.join(dir, "dlltool.exe")) && fs.existsSync(path.join(dir, "as.exe"));
}

function cargoEnv(cargo) {
  // project-local toolchains get explicit HOMEs; system cargo used as-is.
  const env = Object.assign({}, process.env);
  const c = path.resolve(cargo);
  if (c === path.resolve(BUNDLED_CARGO)) {
    env.RUSTUP_HOME = path.join(ROOT, "platform", "linux", "rustup");
    env.CARGO_HOME = path.join(ROOT, "platform", "linux", "cargo");
  } else if (c === path.resolve(path.join(P_CARGO, "bin", IS_WIN ? "cargo.exe" : "cargo"))) {
    env.RUSTUP_HOME = P_RUSTUP;
    env.CARGO_HOME = P_CARGO;
  }
  setEnvPath(env, [path.dirname(cargo)].concat(EXTRA_PATH, envPath(env)).filter(Boolean).join(path.delimiter));
  return env;
}

function findCargo() {
  // Order: project-local (windows) -> bundled linux -> PATH -> ~/.cargo/bin.
  if (IS_WIN) {
    const pc = path.join(P_CARGO, "bin", "cargo.exe");
    if (fs.existsSync(pc)) return pc;
  }
  if (fs.existsSync(BUNDLED_CARGO)) return BUNDLED_CARGO;
  const exe = IS_WIN ? "cargo.exe" : "cargo";
  for (const d of (process.env.PATH || "").split(path.delimiter)) {
    const c = path.join(d, exe);
    if (fs.existsSync(c)) return c;
  }
  const home = process.env.USERPROFILE || process.env.HOME || "";
  const ru = path.join(home, ".cargo", "bin", exe);
  if (fs.existsSync(ru)) return ru;
  return null;
}

function ask(question) {
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(question, (a) => { rl.close(); resolve(a); });
  });
}

function runPS(cmd) {
  return spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", cmd], { stdio: "inherit" });
}

function download(url, dest) {
  // curl.exe ships with Windows (10 1803+); far faster than Invoke-WebRequest and
  // renders a progress bar. -C - resumes a previous partial download instead of
  // restarting from zero. PowerShell fallback for ancient systems.
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (fs.existsSync(curl)) {
    const r = spawnSync(curl, ["-L", "--fail", "--retry", "3", "-C", "-", "--connect-timeout", "30", "-o", dest, url],
      { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(dest) && fs.statSync(dest).size > 1048576) return true;
    log("curl download failed (exit " + r.status + "); keeping partial file for resume; trying PowerShell fallback...");
  }
  runPS("Invoke-WebRequest -UseBasicParsing '" + url + "' -OutFile '" + dest + "'");
  return fs.existsSync(dest) && fs.statSync(dest).size > 1048576;
}

function mirrorVariants(ghUrl) {
  // Optional extra mirror via SOLOMNI_GH_MIRROR (prefix-proxy shape, e.g.
  // https://ghfast.top/github.com/owner/repo/...). TUNA/fastgit were tried and
  // dropped with evidence: TUNA github-release only mirrors projects that applied
  // for inclusion (404 for everything else), fastgit is shut down.
  const m = process.env.SOLOMNI_GH_MIRROR || "";
  if (!m) return [];
  return [m.replace(/\/+$/, "") + "/" + ghUrl.replace("https://github.com/", "")];
}

function sha256(file) {
  const r = spawnSync("certutil", ["-hashfile", file, "SHA256"], { encoding: "utf8" });
  const m = ((r.stdout || "").match(/^[a-f0-9]{64}$/im) || [])[0];
  return (m || "").toLowerCase();
}

function checkWinlibsHash(zip) {
  // Third-party mirror paths must not silently serve altered content. Pin the
  // upstream hash in WINLIBS_SHA256 once, then every download is verified.
  if (!WINLIBS_SHA256) {
    const h = sha256(zip);
    if (h) {
      log("sha256 " + h);
      log("(pin this in start.js WINLIBS_SHA256 to verify future downloads)");
    }
    return;
  }
  const h = sha256(zip);
  if (h !== WINLIBS_SHA256.toLowerCase()) {
    try { fs.rmSync(zip, { force: true }); } catch (e) { /* remove bad archive */ }
    die("sha256 mismatch (" + (h || "hash unavailable") + " != " + WINLIBS_SHA256 + ") - file deleted. Re-run to download again.");
  }
  log("sha256 verified");
}

function upstreamWinlibsUrl() {
  // Upstream asset names carry versions (winlibs-x86_64-posix-seh-gcc-...zip), so
  // latest/download/winlibs.zip is a guaranteed 404. Resolve the real name via the API.
  const q = [
    "$ErrorActionPreference = 'Stop'",
    "$r = Invoke-RestMethod 'https://api.github.com/repos/brechtsanders/winlibs_mingw/releases/latest'",
    "$a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match 'seh' -and $_.name -match '[.]zip$' -and $_.name -notmatch 'llvm' } | Select-Object -First 1",
    "if (-not $a) { $a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match '[.]zip$' } | Select-Object -First 1 }",
    "Write-Output $a.browser_download_url"
  ].join("; ");
  const got = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", q], { encoding: "utf8" });
  const url = (got.stdout || "").trim();
  return /^https:/.test(url) ? url : null;
}

function probeOnce(url) {
  // One ranged GET (2MB). Resolves { code, speed } - code 000 means the network
  // layer itself failed (DNS/reset/timeout); otherwise the final HTTP status.
  return new Promise((resolve) => {
    const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
    const cmd = IS_WIN ? curl : "curl";
    if (!fs.existsSync(cmd)) { resolve({ code: "000", speed: 0 }); return; }
    const p = spawn(cmd,
      ["-sL", "-r", "0-2097151", "-o", require("os").devNull, "-w", "%{http_code} %{speed_download}",
       "--connect-timeout", "4", "--max-time", "8", url],
      { encoding: "utf8" });
    let out = "";
    p.stdout.on("data", (d) => { out += d; });
    p.on("error", () => resolve({ code: "000", speed: 0 }));
    p.on("close", () => {
      const m = out.trim().match(/^(\d{3}) (\d+(?:\.\d+)?)$/);
      if (m) resolve({ code: m[1], speed: parseFloat(m[2]) || 0 });
      else resolve({ code: "000", speed: 0 });
    });
  });
}

async function probeSpeed(url) {
  // Two tries: transient resets are common on GFW-adjacent routes; a single
  // probe would kill reachable sources half the time.
  const a = await probeOnce(url);
  if (/^2/.test(a.code)) return a;
  const b = await probeOnce(url);
  return /^2/.test(b.code) ? b : a;
}

async function pickFastest(urls) {
  // Speed test every candidate (sequential: parallel streams would share bandwidth
  // and skew each other's numbers on a thin pipe). Fastest wins; listed order is
  // only the tiebreak. Costs a few MB one-time vs a 261MB download.
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (IS_WIN && !fs.existsSync(curl)) return urls[0];
  log("speed-testing " + urls.length + " source(s), 2MB probe each...");
  const speeds = [];
  const codes = [];
  for (const u of urls) {
    const r = await probeSpeed(u);
    speeds.push(r.speed);
    codes.push(r.code);
    log("  " + Math.round(r.speed / 1024) + " KB/s  [http " + r.code + "]  " + u);
  }
  let best = -1;
  for (let i = 0; i < urls.length; i++) {
    if (/^2/.test(codes[i]) && speeds[i] > 0 && (best < 0 || speeds[i] > speeds[best])) best = i;
  }
  if (best < 0) {
    console.error("[start] no source passed the probe (2xx with data). Codes above mean:");
    console.error("  000 = network blocked/reset (set HTTPS_PROXY, or download the zip manually)");
    console.error("  404 = not on this host (wrong tag/asset name, or mirror does not carry it)");
    console.error("  403 = rate limited (wait a bit and re-run)");
    die("no reachable source. Manual: browser-download a winlibs x86_64 seh zip, save as .tools/winlibs.zip, re-run.");
  }
  log("fastest: " + urls[best]);
  return urls[best];
}

function zipLooksValid(zipPath) {
  // Three-state check: true = complete archive with dlltool inside; "partial" =
  // looks like an unfinished download, keep it for resume; false = corrupt content,
  // delete. Structural listing via the system tar (Win10 1803+ ships one).
  const st = fs.statSync(zipPath);
  if (st.size < 10485760) return "partial"; // real winlibs zip is far bigger; resumable fragment
  const t = spawnSync("tar", ["-tf", zipPath], { encoding: "utf8" });
  if (t.error && t.error.code === "ENOENT") return true; // no tar available: size-only, do not delete a possibly-good file
  if (t.status !== 0) return false; // big but unreadable: wrong content, not a resume point
  return /bin[\\/]dlltool\.exe/i.test(t.stdout || "");
}

async function installWinlibs() {
  // Portable MinGW-w64 (binutils provides dlltool.exe). Project-local: .tools/mingw64.
  // No admin, no system PATH change; delete the directory to remove.
  // Source: our GitHub Release mirror of the unmodified upstream zip (GPL: plain
  // redistribution with attribution is permitted), falling back to the upstream URL.
  if (binutilsReady(WINLIBS_BIN)) {
    log("portable MinGW already installed and working: " + WINLIBS_BIN);
    return;
  }
  fs.mkdirSync(TOOLS, { recursive: true });
  const zip = path.join(TOOLS, "winlibs.zip");
  if (fs.existsSync(zip)) {
    const v = zipLooksValid(zip);
    if (v === true) {
      log("using existing " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB)");
    } else if (v === "partial") {
      log("found partial " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB) - will resume.");
    } else {
      log("existing " + zip + " is corrupt (wrong content) - deleting and re-downloading.");
      try { fs.rmSync(zip, { force: true }); } catch (e) { /* re-attempt below anyway */ }
    }
  }
  if (!fs.existsSync(zip)) {
    const rel = REL_BASE.replace(/\/+$/, ""); // tolerate trailing slash in REL_BASE
    const ours = rel + "/winlibs.zip";
    const candidates = [ours].concat(mirrorVariants(ours));
    const upstream = await upstreamWinlibsUrl();
    if (upstream) candidates.push(upstream, ...mirrorVariants(upstream));
    const extraMirror = (process.env.SOLOMNI_GH_MIRROR || "").replace(/\/+$/, "");
    if (extraMirror) candidates.push(extraMirror + "/" + (upstream || ours).replace("https://github.com/", ""));
    const seen = [];
    for (const c of candidates) if (seen.indexOf(c) < 0) seen.push(c);
    const url = await pickFastest(seen);
    log("downloading " + url);
    log("(curl with progress; ~200 MB. Too slow? set SOLOMNI_GH_MIRROR=https://ghfast.top");
    log(" or HTTPS_PROXY=http://127.0.0.1:port, or browser-save the zip as .tools/winlibs.zip)");
    const ok = download(url, zip) ? zipLooksValid(zip) : false;
    if (ok === false) {
      log("download still incomplete or corrupt - re-run to resume, or download manually:");
      die("  get a winlibs x86_64 seh zip from https://github.com/brechtsanders/winlibs_mingw/releases");
    }
    checkWinlibsHash(zip);
  }
  log("extracting (takes a minute)...");
  const tar = spawnSync("tar", ["-xf", zip, "-C", TOOLS], { stdio: "ignore" });
  if (tar.status !== 0) runPS("Expand-Archive -Force '" + zip + "' -DestinationPath '" + TOOLS + "'");
  fs.rmSync(zip, { force: true });
  if (!fs.existsSync(path.join(WINLIBS_BIN, "dlltool.exe"))) die("extracted, but dlltool.exe is not in .tools/mingw64/bin - inspect the .tools directory.");
  log("portable MinGW ready: " + WINLIBS_BIN);
}

async function installRust() {
  log("Rust toolchain not found (required to build Solomni).");
  const ans = await ask("Download and install Rust now via rustup? [y/N] ");
  if (ans.trim().toLowerCase() !== "y") {
    die("aborted. Install Rust manually: https://rustup.rs then re-run.");
  }
  log("installing Rust (project-local, a few hundred MB, once; nothing written outside the project)...");
  if (IS_WIN) {
    // GNU toolchain: self-contained linker, no Visual Studio Build Tools needed.
    // Project-local: RUSTUP_HOME/CARGO_HOME under platform/windows, --no-modify-path
    // so the user's system PATH stays untouched.
    fs.mkdirSync(path.join(ROOT, "platform", "windows"), { recursive: true });
    const initExe = path.join(TOOLS, "rustup-init.exe");
    if (!download("https://win.rustup.rs/x86_64", initExe)) {
      die("rustup-init download failed. Check network / proxy.");
    }
    const init = spawnSync(initExe,
      ["-y", "--default-toolchain", "stable-x86_64-pc-windows-gnu", "--no-modify-path"],
      { stdio: "inherit", env: Object.assign({}, process.env, { RUSTUP_HOME: P_RUSTUP, CARGO_HOME: P_CARGO }) });
    if (init.status !== 0) die("rustup install failed.");
  } else {
    const r = spawnSync("sh", ["-c", "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"],
      { stdio: "inherit" });
    if (r.status !== 0) die("rustup install failed. Check network / curl availability.");
  }
  const cargo = findCargo();
  if (!cargo) die("installed, but cargo still not visible. Reopen the terminal and re-run.");
  log("Rust installed: " + cargo);
  return cargo;
}

function run(cargo) {
  const pass = process.argv.slice(2).filter((a) => a !== "--release");
  const argv = [BIN, "."].concat(pass);
  log(process.argv.includes("-webUI")
    ? "starting Web UI (default 127.0.0.1:3081, open http://127.0.0.1:3081)"
    : "starting CLI (type webui at the prompt for the Web UI, or start with -webUI)");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: ROOT, stdio: "inherit", env: cargoEnv(cargo) });
  if (r.error) die("run failed: " + r.error.message);
  process.exitCode = r.status || 0;
}

(async () => {
  let cargo = findCargo();
  if (!cargo) cargo = await installRust();
  log("platform " + process.platform + " " + process.arch);

  if (IS_WIN) {
    // Preflight BEFORE building: windows-sys fails at compile time without a WORKING
    // dlltool (it must be able to spawn "as.exe"; the toolchain's self-contained copy
    // often lacks one - CreateProcess failure at import-lib generation, see #140704).
    const probe = locateDlltool(cargo);
    if (probe.dir) {
      EXTRA_PATH.push(probe.dir);
      log("dlltool working: " + probe.dir);
    } else {
      if (probe.broken.length) log("dlltool present but unusable (no assembler): " + probe.broken.join(", "));
      else {
        console.error("[start] dlltool.exe NOT found at any fixed location (rust-lang/rust#140704:");
        console.error("[start] windows-sys raw-dylib needs dlltool; rust-mingw on this toolchain lacks it).");
      }
      const ans = await ask("Install portable MinGW now? winlibs ~200MB into project .tools, no admin, no system changes [y/N] ");
      if (ans.trim().toLowerCase() === "y") {
        await installWinlibs();
        EXTRA_PATH.push(WINLIBS_BIN);
      } else {
        console.error("[start] manual alternatives:");
        console.error("  1) MSYS2: winget install MSYS2.MSYS2 ; pacman -S mingw-w64-x86_64-binutils ; add C:/msys64/mingw64/bin to PATH");
        console.error("  2) MSVC: install VS 2022 Build Tools (VC workload) ; rustup default stable-x86_64-pc-windows-msvc");
        console.error("  3) Browser: download winlibs zip manually, save as .tools/winlibs.zip, re-run");
        console.error("     (set SOLOMNI_GH_MIRROR to a GitHub mirror prefix to speed up auto-download)");
      }
    }
  }

  const v = spawnSync(cargo, ["--version"], { cwd: ROOT, env: cargoEnv(cargo), encoding: "utf8" });
  if (v.error || v.status !== 0) die("cargo not runnable: " + (v.error && v.error.message));
  log(v.stdout.trim());
  // Always invoke cargo: it decides what is stale in ~a second. Skipping the build
  // when a binary already existed made the launcher run outdated binaries after
  // source changes.
  if (binLocked(BIN)) {
    log("binary is held by a running Solomni instance - cannot rebuild, starting it as-is.");
    log("close the other window/session to pick up source changes.");
    run(cargo);
    return;
  }
  log(fs.existsSync(BIN) ? "checking build..." : "binary not found, building (first run is slow)...");
  const args = RELEASE ? ["build", "--release"] : ["build"];
  const b = spawnSync(cargo, args, { cwd: ROOT, env: cargoEnv(cargo), stdio: "inherit" });
  if (b.status !== 0) die("build failed (is solomni running in another window? close it and retry)");

  // Rebuild may have replaced the file the lock probe checked; re-verify before use.
  if (!fs.existsSync(BIN)) die("build reported success but no binary at " + path.relative(ROOT, BIN));
  run(cargo);
})();
