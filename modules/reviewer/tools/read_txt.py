#!/usr/bin/env python3
"""读文本文件，带行号输出（本模块工具；参数经 stdin JSON 传入）。"""
import sys, json, os

def main():
    args = json.loads(sys.stdin.read() or "{}")
    path = args.get("path", "")
    if not path:
        sys.exit("缺少参数：path")
    start = int(args.get("start") or 1)
    end = args.get("end")
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        lines = f.read().splitlines()
    end_i = int(end) if end is not None else len(lines)
    start = max(1, start)
    picked = 0
    for no in range(start, min(end_i, len(lines)) + 1):
        print(f"{no:>5} | {lines[no - 1]}")
        picked += 1
    if picked == 0:
        print(f"(区间 {start}-{end_i} 无内容；全文共 {len(lines)} 行)")
    else:
        print(f"(输出 {picked} 行；全文共 {len(lines)} 行)")

if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass
    main()
