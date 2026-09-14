#!/usr/bin/env python3
"""关键词搜索，命中行带行号输出（本模块工具；参数经 stdin JSON 传入）。"""
import sys, json

def main():
    args = json.loads(sys.stdin.read() or "{}")
    path = args.get("path", "")
    keyword = args.get("keyword", "")
    if not path or not keyword:
        sys.exit("缺少参数：path / keyword")
    ignore_case = bool(args.get("ignore_case"))
    needle = keyword.lower() if ignore_case else keyword
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        lines = f.read().splitlines()
    hits = 0
    for no, line in enumerate(lines, 1):
        hay = line.lower() if ignore_case else line
        if needle in hay:
            print(f"{no:>5} | {line}")
            hits += 1
    print(f"(关键词 "{keyword}" 命中 {hits} 行 / 共 {len(lines)} 行)")

if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass
    main()
