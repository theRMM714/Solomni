// modules/indexer/tools/src/indexer.cpp
// indexer：给 corpus.jsonl 建倒排索引、做本地检索、找近似重复。
// 契约：argv[1] 是子命令（build / query / dups）；参数从 stdin 收 JSON；
//       结果写 stdout（UTF-8、紧凑、至多约 40 行）；失败走 stderr + 非零退出码。
// 零第三方依赖：只用 C++17 标准库。

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <iterator>
#include <sstream>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

namespace {

// 退出码：2 = 用法/参数；3 = 文件 IO；4 = JSON 非法；5 = index.bin 损坏。
struct ToolError : std::runtime_error {
  int code;
  ToolError(int c, const std::string& what) : std::runtime_error(what), code(c) {}
};

// ---------------------------------------------------------------- 最小 JSON

struct Json {
  enum class Kind { Null, Bool, Num, Str, Arr, Obj };
  Kind kind = Kind::Null;
  bool boolean = false;
  double number = 0.0;
  std::string str;
  std::vector<Json> arr;
  std::vector<std::pair<std::string, Json>> obj;

  const Json* find(const std::string& key) const {
    if (kind != Kind::Obj) return nullptr;
    for (const auto& kv : obj) {
      if (kv.first == key) return &kv.second;
    }
    return nullptr;
  }
};

void appendUtf8(std::string& out, uint32_t cp) {
  if (cp > 0x10FFFFu) cp = 0xFFFDu;
  if (cp < 0x80u) {
    out.push_back(static_cast<char>(cp));
  } else if (cp < 0x800u) {
    out.push_back(static_cast<char>(0xC0u | (cp >> 6)));
    out.push_back(static_cast<char>(0x80u | (cp & 0x3Fu)));
  } else if (cp < 0x10000u) {
    out.push_back(static_cast<char>(0xE0u | (cp >> 12)));
    out.push_back(static_cast<char>(0x80u | ((cp >> 6) & 0x3Fu)));
    out.push_back(static_cast<char>(0x80u | (cp & 0x3Fu)));
  } else {
    out.push_back(static_cast<char>(0xF0u | (cp >> 18)));
    out.push_back(static_cast<char>(0x80u | ((cp >> 12) & 0x3Fu)));
    out.push_back(static_cast<char>(0x80u | ((cp >> 6) & 0x3Fu)));
    out.push_back(static_cast<char>(0x80u | (cp & 0x3Fu)));
  }
}

class JsonParser {
 public:
  explicit JsonParser(const std::string& s) : s_(s) {}

  Json parse() {
    skipWs();
    if (pos_ >= s_.size()) fail("输入为空，期望一个 JSON 值");
    Json v = parseValue();
    skipWs();
    if (pos_ != s_.size()) fail("JSON 结尾有多余内容");
    return v;
  }

 private:
  const std::string& s_;
  size_t pos_ = 0;

  [[noreturn]] void fail(const std::string& msg) const {
    size_t line = 1;
    for (size_t i = 0; i < pos_ && i < s_.size(); ++i) {
      if (s_[i] == '\n') ++line;
    }
    throw ToolError(4, "JSON 非法：第 " + std::to_string(line) + " 行（字节偏移 " +
                           std::to_string(pos_) + "）：" + msg);
  }

  void skipWs() {
    while (pos_ < s_.size()) {
      const char c = s_[pos_];
      if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
        ++pos_;
      } else {
        break;
      }
    }
  }

  void expect(const char* lit) {
    const size_t n = std::strlen(lit);
    if (pos_ + n > s_.size() || s_.compare(pos_, n, lit) != 0) {
      fail(std::string("期望 \"") + lit + "\"");
    }
    pos_ += n;
  }

  Json parseValue() {
    skipWs();
    if (pos_ >= s_.size()) fail("值缺失");
    const char c = s_[pos_];
    if (c == '{') return parseObject();
    if (c == '[') return parseArray();
    if (c == '"') {
      Json v;
      v.kind = Json::Kind::Str;
      v.str = parseString();
      return v;
    }
    if (c == 't') {
      expect("true");
      Json v;
      v.kind = Json::Kind::Bool;
      v.boolean = true;
      return v;
    }
    if (c == 'f') {
      expect("false");
      Json v;
      v.kind = Json::Kind::Bool;
      v.boolean = false;
      return v;
    }
    if (c == 'n') {
      expect("null");
      Json v;
      v.kind = Json::Kind::Null;
      return v;
    }
    if (c == '-' || (c >= '0' && c <= '9')) return parseNumber();
    fail(std::string("意外的字符 '") + c + "'");
  }

  Json parseNumber() {
    const size_t start = pos_;
    if (pos_ < s_.size() && s_[pos_] == '-') ++pos_;
    while (pos_ < s_.size() && s_[pos_] >= '0' && s_[pos_] <= '9') ++pos_;
    if (pos_ < s_.size() && s_[pos_] == '.') {
      ++pos_;
      while (pos_ < s_.size() && s_[pos_] >= '0' && s_[pos_] <= '9') ++pos_;
    }
    if (pos_ < s_.size() && (s_[pos_] == 'e' || s_[pos_] == 'E')) {
      ++pos_;
      if (pos_ < s_.size() && (s_[pos_] == '+' || s_[pos_] == '-')) ++pos_;
      while (pos_ < s_.size() && s_[pos_] >= '0' && s_[pos_] <= '9') ++pos_;
    }
    const std::string num = s_.substr(start, pos_ - start);
    bool hasDigit = false;
    for (const char ch : num) {
      if (ch >= '0' && ch <= '9') hasDigit = true;
    }
    if (!hasDigit) fail("数字格式非法");
    Json v;
    v.kind = Json::Kind::Num;
    v.number = std::strtod(num.c_str(), nullptr);
    return v;
  }

  uint32_t parseHex4() {
    if (pos_ + 4 > s_.size()) fail("\\u 后面不足 4 位十六进制");
    uint32_t v = 0;
    for (int i = 0; i < 4; ++i) {
      const char ch = s_[pos_++];
      v <<= 4;
      if (ch >= '0' && ch <= '9') {
        v |= static_cast<uint32_t>(ch - '0');
      } else if (ch >= 'a' && ch <= 'f') {
        v |= static_cast<uint32_t>(ch - 'a' + 10);
      } else if (ch >= 'A' && ch <= 'F') {
        v |= static_cast<uint32_t>(ch - 'A' + 10);
      } else {
        fail("\\u 后面不是十六进制数字");
      }
    }
    return v;
  }

  std::string parseString() {
    ++pos_;  // 开引号
    std::string out;
    for (;;) {
      if (pos_ >= s_.size()) fail("字符串没有收尾引号");
      const unsigned char c = static_cast<unsigned char>(s_[pos_]);
      if (c == '"') {
        ++pos_;
        break;
      }
      if (c == '\\') {
        ++pos_;
        if (pos_ >= s_.size()) fail("转义字符不完整");
        const char e = s_[pos_++];
        switch (e) {
          case '"': out.push_back('"'); break;
          case '\\': out.push_back('\\'); break;
          case '/': out.push_back('/'); break;
          case 'b': out.push_back('\b'); break;
          case 'f': out.push_back('\f'); break;
          case 'n': out.push_back('\n'); break;
          case 'r': out.push_back('\r'); break;
          case 't': out.push_back('\t'); break;
          case 'u': {
            uint32_t cp = parseHex4();
            if (cp >= 0xD800u && cp <= 0xDBFFu) {
              if (pos_ + 1 < s_.size() && s_[pos_] == '\\' && s_[pos_ + 1] == 'u') {
                pos_ += 2;
                const uint32_t lo = parseHex4();
                if (lo >= 0xDC00u && lo <= 0xDFFFu) {
                  cp = 0x10000u + ((cp - 0xD800u) << 10) + (lo - 0xDC00u);
                } else {
                  cp = 0xFFFDu;
                  if (!(lo >= 0xD800u && lo <= 0xDFFFu)) appendUtf8(out, lo);
                }
              } else {
                cp = 0xFFFDu;
              }
            } else if (cp >= 0xDC00u && cp <= 0xDFFFu) {
              cp = 0xFFFDu;
            }
            appendUtf8(out, cp);
            break;
          }
          default: fail(std::string("不支持的转义 \\") + e);
        }
        continue;
      }
      if (c < 0x20u) fail("字符串里有未转义的控制字符");
      out.push_back(static_cast<char>(c));
      ++pos_;
    }
    return out;
  }

  Json parseObject() {
    Json v;
    v.kind = Json::Kind::Obj;
    ++pos_;  // '{'
    skipWs();
    if (pos_ < s_.size() && s_[pos_] == '}') {
      ++pos_;
      return v;
    }
    for (;;) {
      skipWs();
      if (pos_ >= s_.size() || s_[pos_] != '"') fail("对象的键必须是字符串");
      std::string key = parseString();
      skipWs();
      if (pos_ >= s_.size() || s_[pos_] != ':') fail("键后面缺少 ':'");
      ++pos_;
      Json val = parseValue();
      v.obj.emplace_back(std::move(key), std::move(val));
      skipWs();
      if (pos_ >= s_.size()) fail("对象没有收尾 '}'");
      if (s_[pos_] == ',') {
        ++pos_;
        continue;
      }
      if (s_[pos_] == '}') {
        ++pos_;
        break;
      }
      fail("对象里期望 ',' 或 '}'");
    }
    return v;
  }

  Json parseArray() {
    Json v;
    v.kind = Json::Kind::Arr;
    ++pos_;  // '['
    skipWs();
    if (pos_ < s_.size() && s_[pos_] == ']') {
      ++pos_;
      return v;
    }
    for (;;) {
      Json item = parseValue();
      v.arr.push_back(std::move(item));
      skipWs();
      if (pos_ >= s_.size()) fail("数组没有收尾 ']'");
      if (s_[pos_] == ',') {
        ++pos_;
        continue;
      }
      if (s_[pos_] == ']') {
        ++pos_;
        break;
      }
      fail("数组里期望 ',' 或 ']'");
    }
    return v;
  }
};

// ---------------------------------------------------------------- UTF-8 与分词

// 解码一个 UTF-8 码点；非法字节按单字节吞掉并返回 false（cp 为该字节值）。
bool decodeUtf8(const std::string& s, size_t& i, uint32_t& cp) {
  const unsigned char b0 = static_cast<unsigned char>(s[i]);
  if (b0 < 0x80u) {
    cp = b0;
    ++i;
    return true;
  }
  size_t need = 0;
  uint32_t v = 0;
  if ((b0 & 0xE0u) == 0xC0u) {
    need = 1;
    v = b0 & 0x1Fu;
  } else if ((b0 & 0xF0u) == 0xE0u) {
    need = 2;
    v = b0 & 0x0Fu;
  } else if ((b0 & 0xF8u) == 0xF0u) {
    need = 3;
    v = b0 & 0x07u;
  } else {
    cp = b0;
    ++i;
    return false;
  }
  if (i + need >= s.size()) {
    cp = b0;
    ++i;
    return false;
  }
  for (size_t k = 1; k <= need; ++k) {
    const unsigned char b = static_cast<unsigned char>(s[i + k]);
    if ((b & 0xC0u) != 0x80u) {
      cp = b0;
      ++i;
      return false;
    }
    v = (v << 6) | (b & 0x3Fu);
  }
  i += need + 1;
  cp = v;
  return true;
}

// CJK 区间：扩展 A、基本区、兼容表意、平假名+片假名、谚文音节。
bool isCjk(uint32_t cp) {
  return (cp >= 0x3400u && cp <= 0x4DBFu) || (cp >= 0x4E00u && cp <= 0x9FFFu) ||
         (cp >= 0xF900u && cp <= 0xFAFFu) || (cp >= 0x3040u && cp <= 0x30FFu) ||
         (cp >= 0xAC00u && cp <= 0xD7AFu);
}

// 分词：ASCII 字母数字（转小写、长度 >= 2）+ CJK 相邻两码点的二元组。
std::vector<std::string> tokenize(const std::string& text) {
  std::vector<std::string> out;
  std::string ascii;
  size_t i = 0;
  bool havePrev = false;
  uint32_t prev = 0;
  while (i < text.size()) {
    uint32_t cp = 0;
    decodeUtf8(text, i, cp);
    if (cp < 0x80u && std::isalnum(static_cast<unsigned char>(cp)) != 0) {
      ascii.push_back(static_cast<char>(std::tolower(static_cast<unsigned char>(cp))));
      havePrev = false;
      continue;
    }
    if (!ascii.empty()) {
      if (ascii.size() >= 2) out.push_back(ascii);
      ascii.clear();
    }
    if (isCjk(cp)) {
      if (havePrev) {
        std::string tok;
        appendUtf8(tok, prev);
        appendUtf8(tok, cp);
        out.push_back(std::move(tok));
      }
      prev = cp;
      havePrev = true;
    } else {
      havePrev = false;
    }
  }
  if (ascii.size() >= 2) out.push_back(ascii);
  return out;
}

std::unordered_map<std::string, uint32_t> countTokens(const std::vector<std::string>& toks) {
  std::unordered_map<std::string, uint32_t> counts;
  counts.reserve(toks.size() * 2 + 1);
  for (const std::string& t : toks) ++counts[t];
  return counts;
}

size_t countCodePoints(const std::string& s) {
  size_t n = 0;
  size_t i = 0;
  while (i < s.size()) {
    uint32_t cp = 0;
    decodeUtf8(s, i, cp);
    ++n;
  }
  return n;
}

// 正文前 160 个码点。
std::string excerptOf(const std::string& text) {
  std::string out;
  size_t i = 0;
  size_t n = 0;
  while (i < text.size() && n < 160) {
    const size_t begin = i;
    uint32_t cp = 0;
    decodeUtf8(text, i, cp);
    out.append(text, begin, i - begin);
    ++n;
  }
  return out;
}

// 输出用：把换行/制表压成空格，保证一条结果就是一行。
std::string oneLine(const std::string& s) {
  std::string out = s;
  for (char& c : out) {
    if (c == '\n' || c == '\r' || c == '\t') c = ' ';
  }
  return out;
}

// ---------------------------------------------------------------- 文件与字节序

std::string readTextFile(const std::string& path, int errCode) {
  std::ifstream in(std::filesystem::u8path(path), std::ios::binary);
  if (!in) throw ToolError(errCode, "打不开文件：" + path);
  std::ostringstream ss;
  ss << in.rdbuf();
  if (in.bad()) throw ToolError(errCode, "读取文件失败：" + path);
  return ss.str();
}

void writeFile(const std::string& path, const std::string& data) {
  const std::filesystem::path p = std::filesystem::u8path(path);
  if (p.has_parent_path()) {
    std::error_code ec;
    std::filesystem::create_directories(p.parent_path(), ec);
    if (ec) {
      throw ToolError(3, "建不了输出目录：" + p.parent_path().u8string() + "（" + ec.message() + "）");
    }
  }
  std::ofstream out(p, std::ios::binary | std::ios::trunc);
  if (!out) throw ToolError(3, "写不了文件：" + path);
  out.write(data.data(), static_cast<std::streamsize>(data.size()));
  if (!out) throw ToolError(3, "写文件失败：" + path);
}

void putU32(std::string& out, uint32_t v) {
  out.push_back(static_cast<char>(v & 0xFFu));
  out.push_back(static_cast<char>((v >> 8) & 0xFFu));
  out.push_back(static_cast<char>((v >> 16) & 0xFFu));
  out.push_back(static_cast<char>((v >> 24) & 0xFFu));
}

uint32_t getU32(const std::string& in, size_t& pos) {
  if (pos + 4 > in.size()) throw ToolError(5, "index.bin 截断：读 uint32 时越过末尾");
  const uint32_t v = static_cast<uint32_t>(static_cast<unsigned char>(in[pos])) |
                     (static_cast<uint32_t>(static_cast<unsigned char>(in[pos + 1])) << 8) |
                     (static_cast<uint32_t>(static_cast<unsigned char>(in[pos + 2])) << 16) |
                     (static_cast<uint32_t>(static_cast<unsigned char>(in[pos + 3])) << 24);
  pos += 4;
  return v;
}

void putStr(std::string& out, const std::string& s) {
  putU32(out, static_cast<uint32_t>(s.size()));
  out += s;
}

std::string getStr(const std::string& in, size_t& pos) {
  const uint32_t n = getU32(in, pos);
  if (pos + n > in.size()) throw ToolError(5, "index.bin 截断：读字符串时越过末尾");
  std::string s = in.substr(pos, n);
  pos += n;
  return s;
}

// ---------------------------------------------------------------- 参数

std::string requireString(const Json& args, const char* name) {
  if (args.kind != Json::Kind::Obj) {
    throw ToolError(2, "stdin 必须是 JSON 对象（收到空输入或别的类型）");
  }
  const Json* v = args.find(name);
  if (v == nullptr || v->kind != Json::Kind::Str) {
    throw ToolError(2, std::string("缺少必填参数 ") + name + "（字符串）");
  }
  if (v->str.empty()) throw ToolError(2, std::string("参数 ") + name + " 不能为空");
  return v->str;
}

// ---------------------------------------------------------------- 语料

struct CorpusDoc {
  std::string rel;
  uint32_t chars = 0;
  std::string text;
};

std::vector<CorpusDoc> loadCorpus(const std::string& path) {
  const std::string data = readTextFile(path, 3);
  std::vector<CorpusDoc> docs;
  size_t pos = 0;
  size_t lineNo = 0;
  while (pos < data.size()) {
    const size_t nl = data.find('\n', pos);
    std::string line =
        (nl == std::string::npos) ? data.substr(pos) : data.substr(pos, nl - pos);
    pos = (nl == std::string::npos) ? data.size() : nl + 1;
    ++lineNo;
    while (!line.empty() && (line.back() == '\r' || line.back() == ' ' || line.back() == '\t')) {
      line.pop_back();
    }
    if (line.empty()) continue;

    Json v;
    try {
      v = JsonParser(line).parse();
    } catch (const ToolError& e) {
      throw ToolError(4, "corpus.jsonl 第 " + std::to_string(lineNo) + " 行：" + e.what());
    }
    if (v.kind != Json::Kind::Obj) {
      throw ToolError(4, "corpus.jsonl 第 " + std::to_string(lineNo) + " 行不是 JSON 对象");
    }
    const Json* rel = v.find("rel");
    const Json* text = v.find("text");
    if (rel == nullptr || rel->kind != Json::Kind::Str) {
      throw ToolError(4, "corpus.jsonl 第 " + std::to_string(lineNo) + " 行缺字符串字段 rel");
    }
    if (text == nullptr || text->kind != Json::Kind::Str) {
      throw ToolError(4, "corpus.jsonl 第 " + std::to_string(lineNo) + " 行缺字符串字段 text");
    }
    CorpusDoc d;
    d.rel = rel->str;
    d.text = text->str;
    const Json* ch = v.find("chars");
    if (ch != nullptr && ch->kind == Json::Kind::Num && ch->number >= 0) {
      const double capped = ch->number > 4294967295.0 ? 4294967295.0 : ch->number;
      d.chars = static_cast<uint32_t>(capped);
    } else {
      d.chars = static_cast<uint32_t>(countCodePoints(d.text));
    }
    docs.push_back(std::move(d));
  }
  return docs;
}

// ---------------------------------------------------------------- 输出行预算

// stdout 至多约 40 行；超出部分如实报告截断了多少行。
class Emitter {
 public:
  explicit Emitter(int maxLines) : maxLines_(maxLines) {}

  void line(const std::string& s) {
    if (lines_ < maxLines_) {
      std::cout << s << "\n";
      ++lines_;
    } else {
      ++skipped_;
    }
  }

  void finish() {
    if (skipped_ > 0) {
      std::cout << "（已截断：还有 " << skipped_ << " 行未显示）\n";
    }
  }

 private:
  int maxLines_;
  int lines_ = 0;
  int skipped_ = 0;
};

// ---------------------------------------------------------------- build

int cmdBuild(const Json& args) {
  const std::string corpusPath = requireString(args, "corpus");
  const std::string outPath = requireString(args, "out");

  const std::vector<CorpusDoc> docs = loadCorpus(corpusPath);

  std::unordered_map<std::string, std::vector<std::pair<uint32_t, uint32_t>>> postings;
  std::vector<std::string> rels;
  std::vector<uint32_t> chars;
  std::vector<std::string> excerpts;
  rels.reserve(docs.size());
  chars.reserve(docs.size());
  excerpts.reserve(docs.size());

  for (size_t i = 0; i < docs.size(); ++i) {
    rels.push_back(docs[i].rel);
    chars.push_back(docs[i].chars);
    excerpts.push_back(excerptOf(docs[i].text));
    const auto counts = countTokens(tokenize(docs[i].text));
    for (const auto& kv : counts) {
      postings[kv.first].emplace_back(static_cast<uint32_t>(i), kv.second);
    }
  }

  std::vector<std::string> terms;
  terms.reserve(postings.size());
  for (const auto& kv : postings) terms.push_back(kv.first);
  std::sort(terms.begin(), terms.end());

  std::string buf;
  buf.append("IDX1", 4);
  putU32(buf, static_cast<uint32_t>(rels.size()));
  putU32(buf, static_cast<uint32_t>(terms.size()));
  for (size_t i = 0; i < rels.size(); ++i) {
    putStr(buf, rels[i]);
    putU32(buf, chars[i]);
    putStr(buf, excerpts[i]);
  }
  for (const std::string& t : terms) {
    putStr(buf, t);
    const auto& p = postings[t];
    putU32(buf, static_cast<uint32_t>(p.size()));
    for (const auto& pr : p) {
      putU32(buf, pr.first);
      putU32(buf, pr.second);
    }
  }

  writeFile(outPath, buf);
  std::cout << "已建索引：" << outPath << "\n";
  std::cout << "文档 " << rels.size() << " 篇，词项 " << terms.size() << " 个，文件 " << buf.size()
            << " 字节\n";
  return 0;
}

// ---------------------------------------------------------------- query

int cmdQuery(const Json& args) {
  const std::string indexPath = requireString(args, "index");
  const std::string q = requireString(args, "q");

  uint32_t limit = 10;
  const Json* lim = args.find("limit");
  if (lim != nullptr) {
    if (lim->kind != Json::Kind::Num) throw ToolError(2, "参数 limit 必须是整数");
    if (lim->number < 1.0 || lim->number > 50.0) throw ToolError(2, "参数 limit 超出范围 1..50");
    limit = static_cast<uint32_t>(lim->number);
  }

  const std::string data = readTextFile(indexPath, 3);
  if (data.size() < 4 || data.compare(0, 4, "IDX1") != 0) {
    throw ToolError(5, "不是 indexer 的 index.bin（magic 不匹配）：" + indexPath);
  }
  size_t pos = 4;
  const uint32_t docCount = getU32(data, pos);
  const uint32_t termCount = getU32(data, pos);

  std::vector<std::string> rels(docCount);
  std::vector<std::string> excerpts(docCount);
  for (uint32_t i = 0; i < docCount; ++i) {
    rels[i] = getStr(data, pos);
    getU32(data, pos);  // chars（当前仅存档，检索输出不用）
    excerpts[i] = getStr(data, pos);
  }
  std::unordered_map<std::string, std::vector<std::pair<uint32_t, uint32_t>>> postings;
  postings.reserve(static_cast<size_t>(termCount) * 2 + 1);
  for (uint32_t i = 0; i < termCount; ++i) {
    std::string term = getStr(data, pos);
    const uint32_t n = getU32(data, pos);
    auto& v = postings[term];
    v.reserve(n);
    for (uint32_t k = 0; k < n; ++k) {
      const uint32_t docId = getU32(data, pos);
      const uint32_t tf = getU32(data, pos);
      v.emplace_back(docId, tf);
    }
  }
  if (pos != data.size()) throw ToolError(5, "index.bin 尾部有多余字节，疑似损坏");

  const std::vector<std::string> qtoks = tokenize(q);
  std::vector<std::string> uniq;
  for (const std::string& t : qtoks) {
    if (std::find(uniq.begin(), uniq.end(), t) == uniq.end()) uniq.push_back(t);
  }

  if (uniq.empty()) {
    std::cout << "查询没有可检索的 token（q 太短或全是标点）：" << oneLine(q) << "\n";
    return 0;
  }

  std::unordered_map<uint32_t, uint32_t> matched;
  for (const std::string& t : uniq) {
    const auto it = postings.find(t);
    if (it == postings.end()) continue;
    for (const auto& pr : it->second) matched[pr.first] += 1u;
  }

  if (matched.empty()) {
    std::cout << "没有命中：索引里没有 q 的任何 token（索引文档 " << docCount << " 篇）。\n";
    std::cout << "查询 token 共 " << uniq.size() << " 个（中文需 2 字以上的连续片段才成 token）。\n";
    return 0;
  }

  struct Hit {
    uint32_t doc;
    uint32_t score;
  };
  std::vector<Hit> hits;
  hits.reserve(matched.size());
  for (const auto& kv : matched) hits.push_back({kv.first, kv.second});
  std::sort(hits.begin(), hits.end(), [&](const Hit& a, const Hit& b) {
    if (a.score != b.score) return a.score > b.score;
    return rels[a.doc] < rels[b.doc];
  });

  Emitter em(39);
  em.line("命中 " + std::to_string(hits.size()) + " 篇（索引 " + std::to_string(docCount) +
          " 篇，显示前 " + std::to_string(std::min<size_t>(limit, hits.size())) +
          " 篇）；格式：命中token数 <TAB> rel <TAB> 摘要");
  const size_t show = std::min<size_t>(limit, hits.size());
  for (size_t i = 0; i < show; ++i) {
    em.line(std::to_string(hits[i].score) + "\t" + rels[hits[i].doc] + "\t" +
            oneLine(excerpts[hits[i].doc]));
  }
  if (hits.size() > show) {
    em.line("（limit=" + std::to_string(limit) + "，命中更多未显示）");
  }
  em.finish();
  return 0;
}

// ---------------------------------------------------------------- dups

uint64_t fnv1a64(const std::string& s) {
  uint64_t h = 14695981039346656037ULL;  // FNV-1a 64 offset basis
  for (const unsigned char c : s) {
    h ^= static_cast<uint64_t>(c);
    h *= 1099511628211ULL;  // FNV prime
  }
  return h;
}

// 第 start 起连续 n 个 token 拼成的 shingle（NUL 分隔）的 FNV-1a 64 哈希。
uint64_t shingleHash(const std::vector<std::string>& toks, size_t start, size_t n) {
  std::string joined;
  for (size_t i = 0; i < n; ++i) {
    if (i > 0) joined.push_back('\0');
    joined += toks[start + i];
  }
  return fnv1a64(joined);
}

// bottom-k 草图估计 Jaccard：取两草图并集中最小的 k 个哈希，统计两边都有的比例。
double bottomKJaccard(const std::vector<uint64_t>& a, const std::vector<uint64_t>& b, size_t k) {
  std::vector<uint64_t> uni;
  uni.reserve(a.size() + b.size());
  std::set_union(a.begin(), a.end(), b.begin(), b.end(), std::back_inserter(uni));
  if (uni.empty()) return 0.0;
  if (uni.size() > k) uni.resize(k);
  size_t both = 0;
  for (const uint64_t h : uni) {
    if (std::binary_search(a.begin(), a.end(), h) && std::binary_search(b.begin(), b.end(), h)) {
      ++both;
    }
  }
  return static_cast<double>(both) / static_cast<double>(uni.size());
}

std::string fmt3(double v) {
  std::ostringstream ss;
  ss << std::fixed << std::setprecision(3) << v;
  return ss.str();
}

int cmdDups(const Json& args) {
  const std::string corpusPath = requireString(args, "corpus");

  double threshold = 0.6;
  const Json* th = args.find("threshold");
  if (th != nullptr) {
    if (th->kind != Json::Kind::Num) throw ToolError(2, "参数 threshold 必须是数字");
    if (th->number < 0.0 || th->number > 1.0) throw ToolError(2, "参数 threshold 超出范围 0..1");
    threshold = th->number;
  }

  const std::vector<CorpusDoc> docs = loadCorpus(corpusPath);
  const size_t K = 64;

  std::vector<std::vector<uint64_t>> sketches(docs.size());
  for (size_t i = 0; i < docs.size(); ++i) {
    const std::vector<std::string> toks = tokenize(docs[i].text);
    std::vector<uint64_t> hashes;
    if (toks.size() >= 5) {
      hashes.reserve(toks.size() - 4);
      for (size_t s = 0; s + 5 <= toks.size(); ++s) hashes.push_back(shingleHash(toks, s, 5));
    } else if (!toks.empty()) {
      // 不足 5 个 token：整篇作为单个 shingle，仍然可与别的短篇比出 0/1 相似度。
      hashes.push_back(shingleHash(toks, 0, toks.size()));
    }
    std::sort(hashes.begin(), hashes.end());
    hashes.erase(std::unique(hashes.begin(), hashes.end()), hashes.end());
    if (hashes.size() > K) hashes.resize(K);
    sketches[i] = std::move(hashes);
  }

  struct Pair {
    size_t a;
    size_t b;
    double sim;
  };
  std::vector<Pair> pairs;
  for (size_t i = 0; i < docs.size(); ++i) {
    for (size_t j = i + 1; j < docs.size(); ++j) {
      const double sim = bottomKJaccard(sketches[i], sketches[j], K);
      if (sim >= threshold) pairs.push_back({i, j, sim});
    }
  }
  std::sort(pairs.begin(), pairs.end(), [&](const Pair& x, const Pair& y) {
    if (x.sim != y.sim) return x.sim > y.sim;
    if (docs[x.a].rel != docs[y.a].rel) return docs[x.a].rel < docs[y.a].rel;
    return docs[x.b].rel < docs[y.b].rel;
  });

  Emitter em(39);
  em.line("近似重复 " + std::to_string(pairs.size()) + " 对（阈值 >= " + fmt3(threshold) + "；语料 " +
          std::to_string(docs.size()) + " 篇，每篇取 5-gram bottom-" + std::to_string(K) +
          " 草图）；格式：相似度 <TAB> relA <TAB> relB");
  if (pairs.empty()) {
    em.line("没有达到阈值的近似重复。");
  }
  for (const Pair& p : pairs) {
    em.line(fmt3(p.sim) + "\t" + docs[p.a].rel + "\t" + docs[p.b].rel);
  }
  em.finish();
  return 0;
}

// ---------------------------------------------------------------- 入口

int run(int argc, char** argv) {
  if (argc < 2) {
    throw ToolError(2, "用法：indexer <build|query|dups>，参数从 stdin 收 JSON");
  }
  const std::string cmd = argv[1];

  std::string input;
  {
    std::ostringstream ss;
    ss << std::cin.rdbuf();
    input = ss.str();
  }
  Json args;
  bool blank = true;
  for (const char c : input) {
    if (std::isspace(static_cast<unsigned char>(c)) == 0) {
      blank = false;
      break;
    }
  }
  if (!blank) args = JsonParser(input).parse();

  if (cmd == "build") return cmdBuild(args);
  if (cmd == "query") return cmdQuery(args);
  if (cmd == "dups") return cmdDups(args);
  throw ToolError(2, "未知子命令：" + cmd + "（可用：build / query / dups）");
}

}  // namespace

int main(int argc, char** argv) {
#ifdef _WIN32
  // stdin/stdout 一律按原始字节（UTF-8）处理，避免 Windows 代码页转码。
  _setmode(_fileno(stdin), _O_BINARY);
  _setmode(_fileno(stdout), _O_BINARY);
#endif
  try {
    return run(argc, argv);
  } catch (const ToolError& e) {
    std::cerr << "indexer: " << e.what() << "\n";
    return e.code;
  } catch (const std::exception& e) {
    std::cerr << "indexer: 未预期的失败：" << e.what() << "\n";
    return 1;
  }
}
