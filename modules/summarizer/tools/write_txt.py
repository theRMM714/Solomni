#!/usr/bin/env python3
"""写文本文件（UTF-8），父目录不存在则创建（本模块工具；参数经 stdin JSON 传入）。"""
import sys, json, os

def main():
    args = json.loads(sys.stdin.read() or "{}")
    path = args.get("path", "")
    if not path:
        sys.exit("缺少参数：path")
    content = args.get("content", "")
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    print(f"已写入 {len(content.encode('utf-8'))} 字节 → {path}")

if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass
    main()
