#!/usr/bin/env python3
"""把目录里的文本资料抽成语料清单（本模块工具；参数经 stdin JSON 传入）。

产出 corpus.jsonl：每行一个 JSON 对象（UTF-8，非 ASCII 原样输出），字段：
  rel / path / kind / bytes / chars / lines / headings / links / text
只读原件；回执只有统计与产物路径（正文不回显）。
"""
import json
import os
import re
import sys
from html.parser import HTMLParser

# 一次扫描的规模闸门：单份超过这个字节数就跳过（如实计入摘要，不静默丢）。
MAX_BYTES = 8 * 1024 * 1024
# 扩展名 → kind。
KINDS = {".md": "md", ".markdown": "md", ".txt": "txt", ".html": "html", ".htm": "html", ".json": "json"}
DEFAULT_INCLUDE = "md,txt,html,json"

MD_HEADING = re.compile(r"^(#{1,6})\s+(.*\S)\s*$")
MD_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)")


class Extractor(HTMLParser):
    """把 HTML 抽成纯文本 + h1..h3 标题 + href 列表（script/style 一律丢掉）。"""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self.headings = []
        self.links = []
        self.skip = 0
        self.heading_tag = None

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style"):
            self.skip += 1
        elif tag in ("h1", "h2", "h3"):
            self.heading_tag = tag
        elif tag == "a":
            for k, v in attrs:
                if k == "href" and v:
                    self.links.append(v)
                    break

    def handle_endtag(self, tag):
        if tag in ("script", "style") and self.skip:
            self.skip -= 1
        elif tag in ("h1", "h2", "h3") and self.heading_tag == tag:
            self.heading_tag = None

    def handle_data(self, data):
        if self.skip:
            return
        text = data.strip()
        if text and self.heading_tag:
            self.headings.append(text)
        self.parts.append(data)


def extract_html(raw):
    p = Extractor()
    try:
        p.feed(raw)
        p.close()
    except Exception:
        # 半截 HTML 也要出语料：能抽多少算多少（如实由 chars 反映）。
        pass
    return "".join(p.parts), p.headings, p.links


def extract_markdown(raw):
    headings = []
    for line in raw.splitlines():
        m = MD_HEADING.match(line)
        if m:
            headings.append(m.group(2))
    links = [m.group(1) for m in MD_LINK.finditer(raw)]
    return raw, headings, links


def record(abs_path, rel, kind, raw, size):
    if kind == "html":
        text, headings, links = extract_html(raw)
    elif kind == "md":
        text, headings, links = extract_markdown(raw)
    else:
        text, headings, links = raw, [], []
    return {
        "rel": rel,
        # 路径的书写形式统一用 /（与提示词里给模型的路径语言一致，跨平台可比对）。
        "path": abs_path.replace(os.sep, "/"),
        "kind": kind,
        "bytes": size,
        "chars": len(text),
        "lines": len(text.splitlines()),
        "headings": headings,
        "links": links,
        "text": text,
    }


def main():
    args = json.loads(sys.stdin.read() or "{}")
    root = str(args.get("root") or "").strip()
    out = str(args.get("out") or "").strip()
    if not root:
        sys.exit("缺少参数：root（要扫描的根目录）")
    if not out:
        sys.exit("缺少参数：out（语料清单写到哪个文件）")
    if not os.path.isdir(root):
        sys.exit("root 不是目录：%s" % root)
    include = str(args.get("include") or DEFAULT_INCLUDE)
    want = {("." + e.strip().lstrip(".")).lower() for e in include.split(",") if e.strip()}

    records = []
    skipped_big = 0
    skipped_kind = 0
    unreadable = 0
    for base, dirs, files in os.walk(root):
        # 隐藏目录与 userdata/ 不进语料（后者是模块自己的跨任务私有状态）。
        dirs[:] = [d for d in dirs if not d.startswith(".") and d != "userdata"]
        for name in sorted(files):
            ext = os.path.splitext(name)[1].lower()
            if ext not in want or ext not in KINDS:
                skipped_kind += 1
                continue
            abs_path = os.path.join(base, name)
            try:
                size = os.path.getsize(abs_path)
            except OSError:
                unreadable += 1
                continue
            if size > MAX_BYTES:
                skipped_big += 1
                continue
            try:
                with open(abs_path, "rb") as f:
                    raw = f.read().decode("utf-8", errors="replace")
            except OSError:
                unreadable += 1
                continue
            rel = os.path.relpath(abs_path, root).replace(os.sep, "/")
            records.append(record(abs_path, rel, KINDS[ext], raw, size))

    parent = os.path.dirname(os.path.abspath(out))
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        for r in records:
            f.write(json.dumps(r, ensure_ascii=False, separators=(",", ":")) + "\n")

    by_kind = {}
    total_chars = 0
    total_lines = 0
    rel_links = 0
    ext_links = 0
    for r in records:
        by_kind[r["kind"]] = by_kind.get(r["kind"], 0) + 1
        total_chars += r["chars"]
        total_lines += r["lines"]
        for link in r["links"]:
            if link.startswith(("http://", "https://", "//", "mailto:")):
                ext_links += 1
            else:
                rel_links += 1

    print("语料清单：%s" % out)
    print("文件 %d 份（%s）" % (len(records), " / ".join("%s %d" % (k, by_kind[k]) for k in sorted(by_kind))))
    print("合计 %d 字符 / %d 行" % (total_chars, total_lines))
    top = sorted(records, key=lambda r: r["chars"], reverse=True)[:3]
    if top:
        print("最大的几份：" + "  ".join("%s(%d)" % (r["rel"], r["chars"]) for r in top))
    print("链接 %d 条（相对 %d / 外链 %d）" % (rel_links + ext_links, rel_links, ext_links))
    if skipped_big or unreadable or skipped_kind:
        print(
            "略过：超大 %d / 读不了 %d / 扩展名不在白名单 %d"
            % (skipped_big, unreadable, skipped_kind)
        )


if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass
    main()
