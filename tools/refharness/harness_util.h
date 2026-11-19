// Small helpers for harness.cpp: JSON writer/parser, hex, UTF-8/UTF-16
// conversion, command line access. Independent of the upstream code.
#pragma once

#include <windows.h>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <map>
#include <memory>
#include <set>
#include <stdexcept>
#include <string>
#include <vector>

namespace hu {

using Bytes = std::vector<uint8_t>;

class HarnessError : public std::runtime_error
{
public:
  explicit HarnessError(const std::string& s) : std::runtime_error(s) {}
};

// ---------------------------------------------------------------------------
// encoding helpers

inline std::string toHex(const void* p, size_t n)
{
  static const char* HEX = "0123456789abcdef";
  const uint8_t* b = static_cast<const uint8_t*>(p);
  std::string s;
  s.reserve(n * 2);
  for (size_t i = 0; i < n; i++) {
    s.push_back(HEX[b[i] >> 4]);
    s.push_back(HEX[b[i] & 15]);
  }
  return s;
}

inline std::string toHex(const Bytes& b)
{
  return toHex(b.data(), b.size());
}

inline Bytes fromHex(const std::string& s)
{
  Bytes out;
  std::string t;
  for (char c : s)
    if (!isspace(static_cast<unsigned char>(c)))
      t.push_back(c);
  if (t.size() % 2 != 0)
    throw HarnessError("odd number of hex digits");
  auto val = [](char c) -> int {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    throw HarnessError(std::string("invalid hex digit '") + c + "'");
  };
  for (size_t i = 0; i < t.size(); i += 2)
    out.push_back(static_cast<uint8_t>(val(t[i]) * 16 + val(t[i + 1])));
  return out;
}

// UTF-16 -> UTF-8; lone surrogates are encoded as WTF-8 (3 bytes) so that no
// information is lost (the JSON writer escapes them as \uXXXX instead).
inline std::string utf16ToUtf8(const wchar_t* p, size_t n)
{
  std::string out;
  for (size_t i = 0; i < n; i++) {
    uint32_t c = static_cast<uint16_t>(p[i]);
    if (c >= 0xD800 && c <= 0xDBFF && i + 1 < n &&
        static_cast<uint16_t>(p[i + 1]) >= 0xDC00 &&
        static_cast<uint16_t>(p[i + 1]) <= 0xDFFF) {
      c = 0x10000 + ((c - 0xD800) << 10) + (static_cast<uint16_t>(p[i + 1]) - 0xDC00);
      i++;
    }
    if (c < 0x80)
      out.push_back(static_cast<char>(c));
    else if (c < 0x800) {
      out.push_back(static_cast<char>(0xC0 | (c >> 6)));
      out.push_back(static_cast<char>(0x80 | (c & 0x3F)));
    }
    else if (c < 0x10000) {
      out.push_back(static_cast<char>(0xE0 | (c >> 12)));
      out.push_back(static_cast<char>(0x80 | ((c >> 6) & 0x3F)));
      out.push_back(static_cast<char>(0x80 | (c & 0x3F)));
    }
    else {
      out.push_back(static_cast<char>(0xF0 | (c >> 18)));
      out.push_back(static_cast<char>(0x80 | ((c >> 12) & 0x3F)));
      out.push_back(static_cast<char>(0x80 | ((c >> 6) & 0x3F)));
      out.push_back(static_cast<char>(0x80 | (c & 0x3F)));
    }
  }
  return out;
}

inline std::string utf16ToUtf8(const std::wstring& s)
{
  return utf16ToUtf8(s.c_str(), s.length());
}

// UTF-8 -> UTF-16 (invalid sequences -> U+FFFD); WTF-8 surrogates accepted
inline std::wstring utf8ToUtf16(const std::string& s)
{
  std::wstring out;
  size_t i = 0;
  while (i < s.size()) {
    uint8_t b = static_cast<uint8_t>(s[i]);
    uint32_t c;
    int extra;
    if (b < 0x80) { c = b; extra = 0; }
    else if ((b & 0xE0) == 0xC0) { c = b & 0x1F; extra = 1; }
    else if ((b & 0xF0) == 0xE0) { c = b & 0x0F; extra = 2; }
    else if ((b & 0xF8) == 0xF0) { c = b & 0x07; extra = 3; }
    else { out.push_back(0xFFFD); i++; continue; }
    bool ok = true;
    for (int k = 1; k <= extra; k++) {
      if (i + k >= s.size() || (static_cast<uint8_t>(s[i + k]) & 0xC0) != 0x80) {
        ok = false;
        break;
      }
      c = (c << 6) | (static_cast<uint8_t>(s[i + k]) & 0x3F);
    }
    if (!ok) {
      out.push_back(0xFFFD);
      i++;
      continue;
    }
    i += extra + 1;
    if (c >= 0x10000) {
      c -= 0x10000;
      out.push_back(static_cast<wchar_t>(0xD800 + (c >> 10)));
      out.push_back(static_cast<wchar_t>(0xDC00 + (c & 0x3FF)));
    }
    else
      out.push_back(static_cast<wchar_t>(c));
  }
  return out;
}

inline Bytes utf16leBytes(const std::wstring& s)
{
  Bytes b(s.size() * 2);
  if (!s.empty())
    memcpy(b.data(), s.data(), b.size());
  return b;
}

// ---------------------------------------------------------------------------
// JSON writer

class JsonWriter
{
public:
  void beginObject() { prefix(); m_out.push_back('{'); m_stack.push_back(true); }
  void endObject() { m_stack.pop_back(); m_out.push_back('}'); }
  void beginArray() { prefix(); m_out.push_back('['); m_stack.push_back(true); }
  void endArray() { m_stack.pop_back(); m_out.push_back(']'); }

  JsonWriter& key(const char* k)
  {
    prefix();
    writeString(k, strlen(k));
    m_out.push_back(':');
    m_afterKey = true;
    return *this;
  }

  void str(const std::string& utf8) { prefix(); writeString(utf8.data(), utf8.size()); }
  void wstr(const wchar_t* p, size_t n) { prefix(); writeWString(p, n); }
  void wstr(const std::wstring& s) { wstr(s.c_str(), s.length()); }
  void wstr(const wchar_t* p) { wstr(p, p ? wcslen(p) : 0); }
  void hex(const void* p, size_t n) { str(toHex(p, n)); }
  void hex(const Bytes& b) { str(toHex(b)); }
  void boolean(bool v) { prefix(); m_out += v ? "true" : "false"; }
  void null() { prefix(); m_out += "null"; }
  void num(long long v) { prefix(); m_out += std::to_string(v); }
  void unum(unsigned long long v) { prefix(); m_out += std::to_string(v); }
  void dbl(double v)
  {
    prefix();
    if (v != v || v > 1.7976931348623157e308 || v < -1.7976931348623157e308) {
      char buf[64];
      snprintf(buf, sizeof(buf), "\"%g\"", v);
      m_out += buf;
      return;
    }
    char buf[64];
    snprintf(buf, sizeof(buf), "%.17g", v);
    // make sure it parses as a float in strict readers
    std::string s(buf);
    if (s.find_first_of(".eEn") == std::string::npos)
      s += ".0";
    m_out += s;
  }
  // double plus its IEEE-754 bit pattern (big-endian hex, like "%016llx")
  void dblWithBits(const char* name, double v)
  {
    key(name).dbl(v);
    uint64_t bits;
    memcpy(&bits, &v, 8);
    char buf[32];
    snprintf(buf, sizeof(buf), "%016llx", static_cast<unsigned long long>(bits));
    std::string k = std::string(name) + "_bits";
    key(k.c_str()).str(buf);
  }
  void raw(const std::string& json) { prefix(); m_out += json; }

  const std::string& result() const { return m_out; }

private:
  std::string m_out;
  std::vector<bool> m_stack;   // true = first element in container
  bool m_afterKey = false;

  void prefix()
  {
    if (m_afterKey) {
      m_afterKey = false;
      return;
    }
    if (!m_stack.empty()) {
      if (!m_stack.back())
        m_out.push_back(',');
      m_stack.back() = false;
    }
  }

  void escapeChar(uint32_t c)
  {
    char buf[8];
    switch (c) {
    case '"': m_out += "\\\""; break;
    case '\\': m_out += "\\\\"; break;
    case '\n': m_out += "\\n"; break;
    case '\r': m_out += "\\r"; break;
    case '\t': m_out += "\\t"; break;
    case '\b': m_out += "\\b"; break;
    case '\f': m_out += "\\f"; break;
    default:
      snprintf(buf, sizeof(buf), "\\u%04x", c);
      m_out += buf;
    }
  }

  void writeString(const char* p, size_t n)
  {
    m_out.push_back('"');
    for (size_t i = 0; i < n; i++) {
      unsigned char c = static_cast<unsigned char>(p[i]);
      if (c < 0x20 || c == '"' || c == '\\')
        escapeChar(c);
      else
        m_out.push_back(static_cast<char>(c));
    }
    m_out.push_back('"');
  }

  // UTF-16 -> JSON string (UTF-8; lone surrogates as \uXXXX escapes)
  void writeWString(const wchar_t* p, size_t n)
  {
    m_out.push_back('"');
    for (size_t i = 0; i < n; i++) {
      uint32_t c = static_cast<uint16_t>(p[i]);
      bool pair = c >= 0xD800 && c <= 0xDBFF && i + 1 < n &&
        static_cast<uint16_t>(p[i + 1]) >= 0xDC00 &&
        static_cast<uint16_t>(p[i + 1]) <= 0xDFFF;
      if (pair) {
        m_out += utf16ToUtf8(p + i, 2);
        i++;
      }
      else if (c >= 0xD800 && c <= 0xDFFF)
        escapeChar(c);
      else if (c < 0x20 || c == '"' || c == '\\')
        escapeChar(c);
      else
        m_out += utf16ToUtf8(p + i, 1);
    }
    m_out.push_back('"');
  }
};

// ---------------------------------------------------------------------------
// minimal JSON parser (for the db-create spec)

struct JValue;
using JPtr = std::shared_ptr<JValue>;

struct JValue
{
  enum Type { Null, Bool, Number, String, Array, Object } type = Null;
  bool b = false;
  double num = 0;
  bool isInt = false;
  long long i = 0;
  unsigned long long u = 0;   // valid if isInt and non-negative
  std::wstring s;              // UTF-16
  std::vector<JPtr> arr;
  std::vector<std::pair<std::string, JPtr>> obj;

  const JValue* get(const char* k) const
  {
    for (const auto& kv : obj)
      if (kv.first == k)
        return kv.second.get();
    return nullptr;
  }
};

class JsonParser
{
public:
  explicit JsonParser(const std::string& text) : m_s(text) {}

  JPtr parse()
  {
    JPtr v = value();
    ws();
    if (m_p != m_s.size())
      fail("trailing characters");
    return v;
  }

private:
  const std::string& m_s;
  size_t m_p = 0;

  [[noreturn]] void fail(const char* msg)
  {
    throw HarnessError(std::string("JSON parse error at offset ") +
      std::to_string(m_p) + ": " + msg);
  }

  void ws()
  {
    while (m_p < m_s.size() && (m_s[m_p] == ' ' || m_s[m_p] == '\t' ||
        m_s[m_p] == '\n' || m_s[m_p] == '\r'))
      m_p++;
  }

  bool lit(const char* t)
  {
    size_t n = strlen(t);
    if (m_s.compare(m_p, n, t) == 0) {
      m_p += n;
      return true;
    }
    return false;
  }

  unsigned hex4()
  {
    if (m_p + 4 > m_s.size())
      fail("bad \\u escape");
    unsigned v = 0;
    for (int k = 0; k < 4; k++) {
      char c = m_s[m_p++];
      v <<= 4;
      if (c >= '0' && c <= '9') v |= c - '0';
      else if (c >= 'a' && c <= 'f') v |= c - 'a' + 10;
      else if (c >= 'A' && c <= 'F') v |= c - 'A' + 10;
      else fail("bad \\u escape");
    }
    return v;
  }

  std::wstring string()
  {
    if (m_s[m_p] != '"')
      fail("expected string");
    m_p++;
    std::wstring out;
    std::string run;
    auto flush = [&]() {
      if (!run.empty()) {
        out += utf8ToUtf16(run);
        run.clear();
      }
    };
    while (true) {
      if (m_p >= m_s.size())
        fail("unterminated string");
      char c = m_s[m_p++];
      if (c == '"')
        break;
      if (c != '\\') {
        run.push_back(c);
        continue;
      }
      flush();
      if (m_p >= m_s.size())
        fail("bad escape");
      char e = m_s[m_p++];
      switch (e) {
      case '"': out.push_back('"'); break;
      case '\\': out.push_back('\\'); break;
      case '/': out.push_back('/'); break;
      case 'b': out.push_back('\b'); break;
      case 'f': out.push_back('\f'); break;
      case 'n': out.push_back('\n'); break;
      case 'r': out.push_back('\r'); break;
      case 't': out.push_back('\t'); break;
      case 'u': out.push_back(static_cast<wchar_t>(hex4())); break;
      default: fail("bad escape");
      }
    }
    flush();
    return out;
  }

  JPtr value()
  {
    ws();
    if (m_p >= m_s.size())
      fail("unexpected end");
    auto v = std::make_shared<JValue>();
    char c = m_s[m_p];
    if (c == '{') {
      v->type = JValue::Object;
      m_p++;
      ws();
      if (m_s[m_p] == '}') {
        m_p++;
        return v;
      }
      while (true) {
        ws();
        std::string k = utf16ToUtf8(string());
        ws();
        if (m_s[m_p++] != ':')
          fail("expected ':'");
        v->obj.emplace_back(k, value());
        ws();
        if (m_s[m_p] == ',') {
          m_p++;
          continue;
        }
        if (m_s[m_p] == '}') {
          m_p++;
          break;
        }
        fail("expected ',' or '}'");
      }
    }
    else if (c == '[') {
      v->type = JValue::Array;
      m_p++;
      ws();
      if (m_s[m_p] == ']') {
        m_p++;
        return v;
      }
      while (true) {
        v->arr.push_back(value());
        ws();
        if (m_s[m_p] == ',') {
          m_p++;
          continue;
        }
        if (m_s[m_p] == ']') {
          m_p++;
          break;
        }
        fail("expected ',' or ']'");
      }
    }
    else if (c == '"') {
      v->type = JValue::String;
      v->s = string();
    }
    else if (lit("true")) {
      v->type = JValue::Bool;
      v->b = true;
    }
    else if (lit("false")) {
      v->type = JValue::Bool;
      v->b = false;
    }
    else if (lit("null")) {
      v->type = JValue::Null;
    }
    else {
      size_t start = m_p;
      if (m_s[m_p] == '-')
        m_p++;
      while (m_p < m_s.size() && (isdigit(static_cast<unsigned char>(m_s[m_p])) ||
          m_s[m_p] == '.' || m_s[m_p] == 'e' || m_s[m_p] == 'E' ||
          m_s[m_p] == '+' || m_s[m_p] == '-'))
        m_p++;
      std::string t = m_s.substr(start, m_p - start);
      if (t.empty() || t == "-")
        fail("unexpected character");
      v->type = JValue::Number;
      v->num = strtod(t.c_str(), nullptr);
      if (t.find_first_of(".eE") == std::string::npos) {
        v->isInt = true;
        if (t[0] == '-')
          v->i = strtoll(t.c_str(), nullptr, 10);
        else {
          v->u = strtoull(t.c_str(), nullptr, 10);
          v->i = static_cast<long long>(v->u);
        }
      }
    }
    return v;
  }
};

// ---------------------------------------------------------------------------
// command line

class Args
{
public:
  Args(int argc, wchar_t** argv, int first, const std::set<std::wstring>& flagNames)
  {
    for (int i = first; i < argc; i++) {
      std::wstring a = argv[i];
      if (a.size() > 2 && a[0] == '-' && a[1] == '-') {
        std::wstring name = a.substr(2);
        auto eq = name.find('=');
        if (eq != std::wstring::npos) {
          m_opts[name.substr(0, eq)].push_back(name.substr(eq + 1));
          continue;
        }
        if (flagNames.count(name)) {
          m_flags.insert(name);
          continue;
        }
        if (i + 1 >= argc)
          throw HarnessError("option --" + utf16ToUtf8(name) + " needs a value");
        m_opts[name].push_back(argv[++i]);
      }
      else
        m_positional.push_back(a);
    }
  }

  bool has(const wchar_t* n) const { return m_opts.count(n) != 0; }
  bool flag(const wchar_t* n) const { return m_flags.count(n) != 0; }

  std::wstring str(const wchar_t* n, const std::wstring& def = L"") const
  {
    auto it = m_opts.find(n);
    return (it == m_opts.end()) ? def : it->second.back();
  }

  std::wstring req(const wchar_t* n) const
  {
    auto it = m_opts.find(n);
    if (it == m_opts.end())
      throw HarnessError("missing required option --" + utf16ToUtf8(n));
    return it->second.back();
  }

  long long num(const wchar_t* n, long long def) const
  {
    auto it = m_opts.find(n);
    if (it == m_opts.end())
      return def;
    const std::wstring& s = it->second.back();
    wchar_t* end = nullptr;
    long long v = wcstoll(s.c_str(), &end, 0);
    if (end == s.c_str() || *end != '\0')
      throw HarnessError("option --" + utf16ToUtf8(n) + ": not a number");
    return v;
  }

  long long reqNum(const wchar_t* n) const
  {
    if (!has(n))
      throw HarnessError("missing required option --" + utf16ToUtf8(n));
    return num(n, 0);
  }

  const std::vector<std::wstring>& positional() const { return m_positional; }

private:
  std::map<std::wstring, std::vector<std::wstring>> m_opts;
  std::set<std::wstring> m_flags;
  std::vector<std::wstring> m_positional;
};

// file helpers (Windows paths)
inline Bytes readFile(const std::wstring& path)
{
  FILE* f = _wfopen(path.c_str(), L"rb");
  if (!f)
    throw HarnessError("cannot open file " + utf16ToUtf8(path));
  Bytes b;
  uint8_t buf[65536];
  size_t n;
  while ((n = fread(buf, 1, sizeof(buf), f)) > 0)
    b.insert(b.end(), buf, buf + n);
  fclose(f);
  return b;
}

} // namespace hu
