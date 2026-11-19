// refharness: drives the ORIGINAL Password Tech 3.6.1 core sources (compiled
// unmodified from upstream/src) and prints reference results as JSON.
// See README.md for the command reference.
#include <vcl.h>
#include <io.h>
#include <fcntl.h>
#include <cmath>
#include <cxxabi.h>
#include <typeinfo>

#include "harness_util.h"

#include "AESCtrPRNG.h"
#include "CryptUtil.h"
#include "CryptText.h"
#include "FastPRNG.h"
#include "PasswDatabase.h"
#include "PasswGen.h"
#include "RandomPool.h"
#include "Main.h"
#include "Util.h"
#include "base64.h"
#include "chacha.h"
#include "hmac.h"
#include "sha1.h"
#include "sha256.h"
#include "sha512.h"
#include "detrand.h"
#include "Scripting.h"
#include "zxcvbn.h"

using namespace hu;

// "PWGen MP Password Generator" (MPPasswGen.h:37) - MPPasswGen.h itself is a
// GUI form header and cannot be included
static const char MPPG_KEYGEN_SALTSTR[] = "PWGen MP Password Generator";

// MPPasswGen.cpp:61
static const int MPPG_CHARSETS_NUM = 6;
static const char* MPPG_CHARSETS[MPPG_CHARSETS_NUM] =
{ "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
  "ACEFHJKLMNPRTUVWXYabcdefghijkmnopqrstuvwxyz3479",
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
  "ACEFHJKLMNPRTUVWXYabcdefghijkmnopqrstuvwxyz3479!\"#$%&'()*+,-./:;<=>?@[\\]^_`{}~",
  "0123456789abcdef",
  "0123456789ABCDEF"
};

// Main.cpp:129
static const int PASSWFORMAT_MAX_CHARS = 16000;

// boolean options (no value)
static const std::set<std::wstring> FLAG_OPTS = {
  L"include-each-subset", L"exclude-ambig", L"determine-subsets",
  L"lowercase-words", L"add-length", L"hashapass", L"recovery", L"combine",
  L"first-char-not-lc", L"old", L"digest", L"header-only",
  L"utf16-passw", L"exclude-duplicates", L"check-each",
};

static void cmdGen(const Args& a, JsonWriter& j, const wchar_t* pwszForceMode);
static void genMainWindowLoop(const Args& a, JsonWriter& j,
  PasswordGenerator& gen, long long qNumOfPassw);

// ---------------------------------------------------------------------------
// helpers

static WString toWS(const std::wstring& s)
{
  return WString(s.c_str(), static_cast<int>(s.length()));
}

static SecureWString toSWS(const std::wstring& s)
{
  SecureWString r;
  r.AssignStr(s.c_str(), static_cast<word32>(s.length()));
  return r;
}

static std::wstring fromSWS(const SecureWString& s)
{
  return std::wstring(s.c_str(), s.StrLen());
}

// Linux path -> Windows path usable by the upstream code (which only knows
// '\' and ':' as delimiters, like the Delphi RTL): '/' -> '\', an absolute
// Unix path gets the Z: drive prefix, then the path is made absolute.
static std::wstring winPath(const std::wstring& p)
{
  if (p.empty())
    return p;
  std::wstring q = p;
  for (auto& c : q)
    if (c == '/')
      c = '\\';
  if (q.size() >= 1 && q[0] == '\\' && !(q.size() >= 2 && q[1] == '\\'))
    q = L"Z:" + q;
  wchar_t buf[4096];
  DWORD n = GetFullPathNameW(q.c_str(), 4096, buf, nullptr);
  if (n > 0 && n < 4096)
    return std::wstring(buf, n);
  return q;
}

// string option NAME: --NAME STR | --NAME-u16hex HEX (UTF-16LE) |
// --NAME-file FILE (UTF-8 file contents, no BOM handling)
static bool hasWStr(const Args& a, const std::wstring& name)
{
  return a.has(name.c_str()) || a.has((name + L"-u16hex").c_str()) ||
    a.has((name + L"-file").c_str());
}

static std::wstring getWStr(const Args& a, const std::wstring& name,
  const std::wstring& def = L"", bool required = false)
{
  if (a.has((name + L"-u16hex").c_str())) {
    Bytes b = fromHex(utf16ToUtf8(a.str((name + L"-u16hex").c_str())));
    if (b.size() % 2)
      throw HarnessError("--" + utf16ToUtf8(name) + "-u16hex: odd byte count");
    return std::wstring(reinterpret_cast<const wchar_t*>(b.data()), b.size() / 2);
  }
  if (a.has((name + L"-file").c_str())) {
    Bytes b = readFile(winPath(a.str((name + L"-file").c_str())));
    return utf8ToUtf16(std::string(b.begin(), b.end()));
  }
  if (a.has(name.c_str()))
    return a.str(name.c_str());
  if (required)
    throw HarnessError("missing required option --" + utf16ToUtf8(name));
  return def;
}

// byte option NAME: --NAME-hex HEX | --NAME-utf8 STR | --NAME-utf16 STR
static bool hasBytes(const Args& a, const std::wstring& name)
{
  return a.has((name + L"-hex").c_str()) || a.has((name + L"-utf8").c_str()) ||
    a.has((name + L"-utf16").c_str());
}

static Bytes getBytes(const Args& a, const std::wstring& name, bool required = true)
{
  if (a.has((name + L"-hex").c_str()))
    return fromHex(utf16ToUtf8(a.str((name + L"-hex").c_str())));
  if (a.has((name + L"-utf8").c_str())) {
    std::string s = utf16ToUtf8(a.str((name + L"-utf8").c_str()));
    return Bytes(s.begin(), s.end());
  }
  if (a.has((name + L"-utf16").c_str()))
    return utf16leBytes(a.str((name + L"-utf16").c_str()));
  if (required)
    throw HarnessError("missing required option --" + utf16ToUtf8(name) + "-hex");
  return Bytes();
}

static unsigned long long fileTimeToU64(const FILETIME& ft)
{
  return (static_cast<unsigned long long>(ft.dwHighDateTime) << 32) | ft.dwLowDateTime;
}

static FILETIME u64ToFileTime(unsigned long long v)
{
  FILETIME ft;
  ft.dwLowDateTime = static_cast<DWORD>(v & 0xffffffff);
  ft.dwHighDateTime = static_cast<DWORD>(v >> 32);
  return ft;
}

static Bytes sha256Of(const void* p, size_t n)
{
  Bytes h(32);
  sha256(reinterpret_cast<const unsigned char*>(p), n, h.data(), 0);
  return h;
}

// w32 password buffer (as produced by PasswordGenerator) -> UTF-16 exactly as
// the main window does it (Main.cpp:2576: W32CharToWCharInternal in place)
static std::wstring w32BufToWString(const word32* p)
{
  size_t n = 0;
  while (p[n] != 0) n++;
  std::vector<word32> tmp(p, p + n + 1);
  W32CharToWCharInternal(tmp.data());
  return std::wstring(reinterpret_cast<const wchar_t*>(tmp.data()));
}

static void writeW32(JsonWriter& j, const char* name, const w32string& s)
{
  std::vector<word32> tmp(s.begin(), s.end());
  tmp.push_back(0);
  j.key(name).wstr(w32BufToWString(tmp.data()));
}

// ---------------------------------------------------------------------------
// prng-aesctr

static void cmdPrngAesCtr(const Args& a, JsonWriter& j)
{
  AESCtrPRNG prng;
  if (hasBytes(a, L"seed")) {
    Bytes seed = getBytes(a, L"seed");
    prng.Seed(seed.data(), static_cast<word32>(seed.size()));
    j.key("seeding").str("Seed");
  }
  else {
    Bytes key = getBytes(a, L"key");
    Bytes param = getBytes(a, L"param", false);
    prng.SeedWithKey(key.data(), static_cast<word32>(key.size()),
      param.empty() ? nullptr : param.data(), static_cast<word32>(param.size()));
    j.key("seeding").str("SeedWithKey");
  }

  long long skip = a.num(L"skip", 0);
  if (skip > 0) {
    std::vector<word8> buf(65536);
    while (skip > 0) {
      word32 n = static_cast<word32>(std::min<long long>(skip, buf.size()));
      prng.GetData(buf.data(), n);
      skip -= n;
    }
  }

  if (a.has(L"bytes")) {
    long long n = a.reqNum(L"bytes");
    Bytes data(static_cast<size_t>(n));
    if (n > 0)
      prng.GetData(data.data(), static_cast<word32>(n));
    if (a.flag(L"digest")) {
      j.key("data_len").num(n);
      j.key("data_sha256").hex(sha256Of(data.data(), data.size()));
    }
    else
      j.key("data").hex(data);
  }

  if (a.has(L"numrange")) {
    long long m = a.reqNum(L"numrange");
    long long count = a.num(L"count", 1);
    j.key("numrange").num(m);
    j.key("values").beginArray();
    for (long long i = 0; i < count; i++)
      j.unum(prng.GetNumRange(static_cast<word32>(m)));
    j.endArray();
  }

  if (a.has(L"after-bytes")) {
    long long n = a.reqNum(L"after-bytes");
    Bytes data(static_cast<size_t>(n));
    if (n > 0)
      prng.GetData(data.data(), static_cast<word32>(n));
    j.key("after").hex(data);
  }
}

// ---------------------------------------------------------------------------
// hashes / KDF

static void cmdPbkdf2(const Args& a, JsonWriter& j)
{
  Bytes passw = getBytes(a, L"passw");
  Bytes salt = getBytes(a, L"salt", false);
  long long iter = a.num(L"iter", 8192);
  Bytes key(32);
  pbkdf2_256bit(passw.data(), static_cast<word32>(passw.size()),
    salt.data(), static_cast<word32>(salt.size()), key.data(),
    static_cast<word32>(iter));
  j.key("iter").num(iter);
  j.key("key").hex(key);
}

static void cmdHmac(const Args& a, JsonWriter& j, int nBits)
{
  Bytes key = getBytes(a, L"key");
  Bytes data = getBytes(a, L"data", false);
  if (nBits == 256) {
    HMAC::SHA256 h(key.data(), static_cast<word32>(key.size()));
    h.Update(data.data(), static_cast<word32>(data.size()));
    auto r = h.Finish();
    j.key("mac").hex(r.Data(), r.Size());
  }
  else if (nBits == 512) {
    HMAC::SHA512 h(key.data(), static_cast<word32>(key.size()));
    h.Update(data.data(), static_cast<word32>(data.size()));
    auto r = h.Finish();
    j.key("mac").hex(r.Data(), r.Size());
  }
  else {
    Bytes r(20);
    sha1_hmac(key.data(), key.size(), data.data(), data.size(), r.data());
    j.key("mac").hex(r);
  }
}

static void cmdHash(const Args& a, JsonWriter& j, int nBits)
{
  Bytes data;
  if (a.has(L"file"))
    data = readFile(winPath(a.str(L"file")));
  else
    data = getBytes(a, L"data", false);
  if (nBits == 256) {
    Bytes r(32);
    sha256(data.data(), data.size(), r.data(), 0);
    j.key("hash").hex(r);
  }
  else if (nBits == 512) {
    Bytes r(64);
    sha512(data.data(), data.size(), r.data(), 0);
    j.key("hash").hex(r);
  }
  else {
    Bytes r(20);
    sha1(data.data(), data.size(), r.data());
    j.key("hash").hex(r);
  }
}

// ---------------------------------------------------------------------------
// chacha (crypto/chacha.c, D. J. Bernstein's "chacha-ref" with 64-bit nonce)

static void cmdChaCha(const Args& a, JsonWriter& j)
{
  Bytes key = getBytes(a, L"key");
  if (key.size() != 16 && key.size() != 32)
    throw HarnessError("key must be 16 or 32 bytes");
  Bytes iv = getBytes(a, L"iv");
  if (iv.size() != 8)
    throw HarnessError("iv must be 8 bytes");
  Bytes ctr = getBytes(a, L"ctr", false);
  if (!ctr.empty() && ctr.size() != 8)
    throw HarnessError("ctr must be 8 bytes");
  long long rounds = a.num(L"rounds", 20);

  chacha_ctx ctx;
  chacha_keysetup(&ctx, key.data(), static_cast<u32>(key.size() * 8));
  chacha_nrounds(&ctx, static_cast<int>(rounds));
  chacha_ivsetup(&ctx, iv.data(), ctr.empty() ? nullptr : ctr.data());

  Bytes data;
  bool blEncrypt = hasBytes(a, L"data");
  if (blEncrypt)
    data = getBytes(a, L"data");
  else
    data.assign(static_cast<size_t>(a.reqNum(L"bytes")), 0);

  // optional: process in chunks (comma-separated sizes, the rest in one call)
  // to expose the chunking behaviour of chacha_encrypt_bytes (partial blocks
  // discard the remaining keystream)
  std::vector<size_t> chunks;
  if (a.has(L"chunks")) {
    std::wstring s = a.str(L"chunks");
    size_t pos = 0;
    while (pos <= s.size()) {
      size_t e = s.find(',', pos);
      if (e == std::wstring::npos) e = s.size();
      if (e > pos)
        chunks.push_back(static_cast<size_t>(std::stoull(s.substr(pos, e - pos))));
      pos = e + 1;
    }
  }
  Bytes out(data.size());
  size_t off = 0;
  for (size_t c : chunks) {
    c = std::min(c, data.size() - off);
    if (blEncrypt)
      chacha_encrypt_bytes(&ctx, data.data() + off, out.data() + off, static_cast<u32>(c));
    else
      chacha_keystream_bytes(&ctx, out.data() + off, static_cast<u32>(c));
    off += c;
  }
  if (off < data.size()) {
    if (blEncrypt)
      chacha_encrypt_bytes(&ctx, data.data() + off, out.data() + off,
        static_cast<u32>(data.size() - off));
    else
      chacha_keystream_bytes(&ctx, out.data() + off, static_cast<u32>(data.size() - off));
  }
  j.key("rounds").num(rounds);
  j.key(blEncrypt ? "ciphertext" : "keystream").hex(out);
}

// ---------------------------------------------------------------------------
// password generator

// seeds an AESCtrPRNG as requested; returns description
static std::string seedGenerator(const Args& a, AESCtrPRNG& prng)
{
  if (hasWStr(a, L"seed-passw")) {
    // Main.cpp:4215-4245 (Tools | Deterministic Random Generator | Set up)
    SecureWString sPassw = toSWS(getWStr(a, L"seed-passw"));
    SecureMem<word8> key(AESCtrPRNG::KEY_SIZE);
    sha256_hmac(sPassw.Bytes(), sPassw.StrLenBytes(),
      reinterpret_cast<const word8*>(MPPG_KEYGEN_SALTSTR),
      sizeof(MPPG_KEYGEN_SALTSTR) - 1, key.Data(), 0);
    prng.SeedWithKey(key, key.Size(), nullptr, 0);
    return "seed-passw";
  }
  if (hasBytes(a, L"seed")) {
    Bytes seed = getBytes(a, L"seed");
    prng.Seed(seed.data(), static_cast<word32>(seed.size()));
    return "seed";
  }
  if (hasBytes(a, L"key")) {
    Bytes key = getBytes(a, L"key");
    Bytes param = getBytes(a, L"param", false);
    prng.SeedWithKey(key.data(), static_cast<word32>(key.size()),
      param.empty() ? nullptr : param.data(), static_cast<word32>(param.size()));
    return "key";
  }
  throw HarnessError("no seed given (--seed-passw, --seed-hex or --key-hex)");
}

static void writeGenInfo(JsonWriter& j, PasswordGenerator& gen)
{
  j.key("custom_charset_type").num(static_cast<int>(gen.CustomCharSetType));
  j.dblWithBits("custom_charset_entropy", gen.CustomCharSetEntropy);
  j.key("custom_charset_unique_size").num(gen.CustomCharSetUniqueSize);
  j.key("custom_charset_size").num(static_cast<long long>(gen.CustomCharSetW32.length()));
  writeW32(j, "custom_charset", gen.CustomCharSetW32);
  j.key("word_list_size").num(gen.WordListSize);
  j.dblWithBits("word_list_entropy", gen.WordListEntropy);
  j.dblWithBits("phonetic_entropy", gen.PhoneticEntropy);
}

// Sets up a PasswordGenerator in the same order as the main window does at
// startup (LoadConfig: separators, trigram file, word list, character set).
static void setupGenerator(const Args& a, JsonWriter& j, PasswordGenerator& gen)
{
  gen.WordSeparator = WStringToW32String(toWS(getWStr(a, L"word-sep")));
  gen.WordCharSeparator = WStringToW32String(toWS(getWStr(a, L"word-char-sep")));

  if (a.has(L"trigram")) {
    int r = gen.LoadTrigramFile(toWS(winPath(a.str(L"trigram"))));
    j.key("trigram_load_result").num(r);
  }

  if (a.has(L"wordlist")) {
    int r = gen.LoadWordListFile(toWS(winPath(a.str(L"wordlist"))),
      // app defaults: Main.cpp:922-929
      static_cast<int>(a.num(L"min-word-len", 1)),
      static_cast<int>(a.num(L"max-word-len", WORDLIST_MAX_WORDLEN)),
      a.flag(L"lowercase-words"));
    j.key("wordlist_load_result").num(r);
  }

  if (hasWStr(a, L"charset") || hasWStr(a, L"ambig") || hasWStr(a, L"symbols") ||
      a.flag(L"include-each-subset") || a.flag(L"exclude-ambig") ||
      a.flag(L"determine-subsets")) {
    WString sResult = gen.SetupCharSets(
      toWS(getWStr(a, L"charset", L"<AZ><az><09>")),
      toWS(getWStr(a, L"ambig")),
      toWS(getWStr(a, L"symbols")),
      a.flag(L"include-each-subset"),
      a.flag(L"exclude-ambig"),
      a.flag(L"determine-subsets"));
    j.key("setup_charsets_result").wstr(sResult.c_str(), sResult.Length());
  }
}

// Transparent wrapper that counts the bytes drawn from the generator (the
// byte stream is identical: AESCtrPRNG::GetByte() and GetData() share one
// buffer, the GetWordNN() defaults call GetData()).
class CountingRng : public RandomGenerator
{
public:
  explicit CountingRng(RandomGenerator& inner) : m_inner(inner) {}
  void Seed(const void* p, word32 n) override { m_inner.Seed(p, n); }
  void GetData(void* p, word32 n) override { m_qBytes += n; m_inner.GetData(p, n); }
  word8 GetByte(void) override { m_qBytes++; return m_inner.GetByte(); }
  word64 Count() const { return m_qBytes; }
private:
  RandomGenerator& m_inner;
  word64 m_qBytes = 0;
};

static void cmdGen(const Args& a, JsonWriter& j, const wchar_t* pwszForceMode)
{
  AESCtrPRNG prngInner;
  j.key("seeding").str(seedGenerator(a, prngInner));
  CountingRng prng(prngInner);

  PasswordGenerator gen(&prng);
  setupGenerator(a, j, gen);
  writeGenInfo(j, gen);

  std::wstring sMode = pwszForceMode ? pwszForceMode : a.str(L"mode", L"password");
  long long count = a.num(L"count", 1);
  int nFlags = static_cast<int>(a.num(L"flags", 0));

  j.key("mode").str(utf16ToUtf8(sMode));
  if (sMode == L"main") {
    // writes its own "passwords" array
    genMainWindowLoop(a, j, gen, count);
  }
  else
    j.key("passwords").beginArray();

  if (sMode == L"main")
    ;
  else if (sMode == L"password" || sMode == L"phonetic") {
    int nLength = static_cast<int>(a.num(L"length", 16));
    SecureW32String sChars(nLength + 1);   // Main.cpp:2337
    for (long long i = 0; i < count; i++) {
      int nLen = (sMode == L"password") ?
        gen.GetPassword(sChars, nLength, nFlags) :
        gen.GetPhoneticPassw(sChars, nLength, nFlags);
      j.beginObject();
      j.key("len").num(nLen);
      j.key("password").wstr(w32BufToWString(sChars));
      j.endObject();
    }
  }
  else if (sMode == L"passphrase") {
    int nWords = static_cast<int>(a.num(L"words", 4));
    w32string sCharsIn = WStringToW32String(toWS(getWStr(a, L"chars")));
    SecureW32String sWords;
    for (long long i = 0; i < count; i++) {
      sWords.BufferedGrow(static_cast<word32>(sCharsIn.length()) + nWords * 10 + 1);
      sWords.SetClearMark(0);
      int nNetWordsLen = 0;
      int nLen = gen.GetPassphrase(sWords, nWords,
        sCharsIn.empty() ? nullptr : sCharsIn.c_str(),
        static_cast<int>(sCharsIn.length()), nFlags, &nNetWordsLen);
      j.beginObject();
      j.key("len").num(nLen);
      j.key("net_words_len").num(nNetWordsLen);
      j.key("password").wstr(w32BufToWString(sWords));
      j.endObject();
    }
  }
  else if (sMode == L"format") {
    w32string sFormat = WStringToW32String(toWS(getWStr(a, L"format", L"", true)));
    w32string sPasswIn = WStringToW32String(toWS(getWStr(a, L"format-passw")));
    bool blWithPassw = hasWStr(a, L"format-passw");
    SecureW32String sFormatted(PASSWFORMAT_MAX_CHARS + 1);   // Main.cpp:2380
    sFormatted.SetClearMark(0);
    for (long long i = 0; i < count; i++) {
      double dSec = 0;
      w32string sInvalidSpec;
      int nPasswUsed = 0;
      std::vector<word32> passwBuf(sPasswIn.begin(), sPasswIn.end());
      passwBuf.push_back(0);
      int nLen = gen.GetFormatPassw(sFormatted, sFormat, nFlags,
        blWithPassw ? passwBuf.data() : nullptr, &nPasswUsed, &sInvalidSpec, &dSec);
      j.beginObject();
      j.key("len").num(nLen);
      j.key("password").wstr(w32BufToWString(sFormatted));
      j.dblWithBits("security", dSec);
      j.key("passw_used").num(nPasswUsed);
      writeW32(j, "invalid_spec", sInvalidSpec);
      j.endObject();
    }
  }
  else
    throw HarnessError("unknown --mode");

  if (sMode != L"main")
    j.endArray();
  j.key("rng_bytes_consumed").unum(prng.Count());
  j.key("prng_next_bytes").hex([&]() {
    Bytes b(8);
    prngInner.GetData(b.data(), 8);
    return b;
  }());
}

// Re-implementation of the core of TMainForm::GeneratePassw()
// (Main.cpp:2220-2700) for a single destination: characters -> words ->
// format -> Lua script, with the same buffers, flag adjustments, RNG call
// order and entropy bookkeeping. The script runs synchronously (the app uses
// a worker thread but waits for each result, so the call order is the same).
// Options: --length (include chars), --words (include words), --format,
// --passw-flags, --passphr-flags, --format-flags (raw PASSW_/PASSPHR_/
// PASSFORMAT_ flags), --combine, --first-char-not-lc, --passphr-len SPEC,
// --exclude-duplicates, --check-each, --script FILE, --advanced-flags N
// (PASSWOPTION_* value passed to the script's init()), --dest N.
static void genMainWindowLoop(const Args& a, JsonWriter& j, PasswordGenerator& gen,
  long long qNumOfPassw)
{
  int nCharsLen = static_cast<int>(a.num(L"length", 0));
  int nNumOfWords = static_cast<int>(a.num(L"words", 0));
  WString sFormatText = toWS(getWStr(a, L"format"));
  w32string sFormatPassw = WStringToW32String(sFormatText);
  int nPasswFlags = static_cast<int>(a.num(L"passw-flags", 0));
  int nPassphrFlags = static_cast<int>(a.num(L"passphr-flags", 0));
  int nFormatFlags = static_cast<int>(a.num(L"format-flags", 0));
  bool blFirstCharNotLC = a.flag(L"first-char-not-lc");
  bool blEachCharOnlyOnce = nPasswFlags & PASSW_FLAG_EACHCHARONLYONCE;
  bool blEachWordOnlyOnce = nPassphrFlags & PASSPHR_FLAG_EACHWORDONLYONCE;
  bool blExcludeDuplicates = a.flag(L"exclude-duplicates");
  int nCharSetSize = 0;

  // Main.cpp:2230 ff.
  if (nCharsLen != 0) {
    nCharSetSize = gen.CustomCharSetW32.length();
    if (blEachCharOnlyOnce && nCharsLen > gen.CustomCharSetUniqueSize)
      nCharsLen = gen.CustomCharSetUniqueSize;
  }

  int nPassphrMinLength = -1, nPassphrMaxLength = -1;
  bool blPassphrLenAllChars = false;
  if (nNumOfWords != 0) {
    if (a.has(L"passphr-len")) {
      WString sText = toWS(a.str(L"passphr-len"));
      // verbatim logic from Main.cpp:2246-2280
      int nSepPos = sText.Pos("*");
      if (nSepPos > 0) {
        blPassphrLenAllChars = true;
        sText.Delete(nSepPos, 1);
      }

      int nLen = sText.Length();
      if (nLen >= 2 && sText[1] == '>') {
        bool blGtEq = nLen >= 3 && sText[2] == '=';
        if (blGtEq)
          nPassphrMinLength = sText.SubString(3, nLen - 2).ToIntDef(-1);
        else
          nPassphrMinLength = sText.SubString(2, nLen - 1).ToIntDef(-2) + 1;
        nPassphrMaxLength = INT_MAX;
      }
      else if (nLen >= 2 && sText[1] == '<') {
        bool blLtEq = nLen >= 3 && sText[2] == '=';
        if (blLtEq)
          nPassphrMaxLength = sText.SubString(3, nLen - 2).ToIntDef(-1);
        else
          nPassphrMaxLength = sText.SubString(2, nLen - 1).ToIntDef(-1) - 1;
        nPassphrMinLength = 0;
      }
      else if ((nSepPos = sText.Pos("-")) >= 2) {
        nPassphrMinLength = sText.SubString(1, nSepPos - 1).ToIntDef(-1);
        nPassphrMaxLength = sText.SubString(nSepPos + 1, nLen - nSepPos).ToIntDef(-1);
      }
      else if (nLen > 0) {
        nPassphrMinLength = nPassphrMaxLength = sText.ToIntDef(-1);
      }

      if (nPassphrMaxLength < nPassphrMinLength)
        std::swap(nPassphrMaxLength, nPassphrMinLength);

      if (nPassphrMinLength < 0 || nPassphrMaxLength < 1)
        throw HarnessError("Passphrase: Invalid length range.");
    }
    if (blEachWordOnlyOnce && nNumOfWords > gen.WordListSize)
      nNumOfWords = gen.WordListSize;
  }

  // Main.cpp:2318 ff.: script loading
  std::unique_ptr<LuaScript> pScript;
  if (a.has(L"script")) {
    pScript.reset(new LuaScript);
    pScript->SetPasswGenerator(&gen);
    pScript->LoadFile(toWS(winPath(a.str(L"script"))));
    j.key("script_flags").num(pScript->Flags);
  }

  if (nCharsLen == 0 && nNumOfWords == 0 && sFormatPassw.empty() && !pScript)
    throw HarnessError("nothing to generate (--length/--words/--format/--script)");

  SecureW32String sChars, sWords, sFormatted;
  SecureWString sFromScript;

  // Main.cpp:2402 ff. (the PASSWOPTION_* -> PASSW_FLAG_* mapping is left to
  // the caller, only the automatic adjustments are reproduced here)
  if (nCharsLen != 0) {
    if (blFirstCharNotLC)
      nPasswFlags |= PASSW_FLAG_FIRSTCHARNOTLC;
    if (nCharSetSize >= 128 && nCharsLen >= 128)
      nPasswFlags |= PASSW_FLAG_CHECKDUPLICATESBYSET;
    if (gen.CustomCharSetType == cstPhoneticUpperCase)
      nPasswFlags |= PASSW_FLAG_PHONETICUPPERCASE;
    else if (gen.CustomCharSetType == cstPhoneticMixedCase)
      nPasswFlags |= PASSW_FLAG_PHONETICMIXEDCASE;
    sChars.New(nCharsLen + 1);
  }
  if (nNumOfWords != 0) {
    if (a.flag(L"combine"))
      nPassphrFlags |= PASSPHR_FLAG_COMBINEWCH;
    if (nPassphrFlags & PASSPHR_FLAG_COMBINEWCH)
      nPasswFlags &= ~PASSW_FLAG_FIRSTCHARNOTLC;
    sWords.BufferedGrow(nCharsLen + nNumOfWords * 10 + 1);
    sWords.SetClearMark(0);
  }
  if (!sFormatPassw.empty()) {
    sFormatted.New(PASSWFORMAT_MAX_CHARS + 1);
    sFormatted.SetClearMark(0);
  }

  // Main.cpp:2392 ff.: script init parameters
  bool blScriptInit = false;
  int nInitGenFlags = 0, nInitCharsLen = nCharsLen, nInitNumOfWords = nNumOfWords;
  WString sInitPasswFormat;
  if (pScript) {
    if (nCharsLen != 0)
      nInitGenFlags |= 1;
    if (nNumOfWords != 0)
      nInitGenFlags |= 2;
    if (!sFormatPassw.empty()) {
      nInitGenFlags |= 4;
      sInitPasswFormat = sFormatText;
    }
    blScriptInit = true;
    if (pScript->Flags & LuaScript::FLAG_STANDALONE_PASSW_GEN) {
      nCharsLen = nNumOfWords = 0;
      sFormatPassw.clear();
    }
    // same generator as the password generator (deterministic mode:
    // g_pRandSrc = key-seeded PRNG)
    pScript->SetRandomGenerator(gen.RandGen);
  }

  j.key("eff_length").num(nCharsLen);
  j.key("eff_words").num(nNumOfWords);
  j.key("passphr_min_len").num(nPassphrMinLength);
  j.key("passphr_max_len").num(nPassphrMaxLength);
  j.key("eff_passw_flags").num(nPasswFlags);
  j.key("eff_passphr_flags").num(nPassphrFlags);

  bool blFirstGen = true;
  bool blKeepPrevPassw = false;
  bool blCheckEachPassw = qNumOfPassw > 1 && a.flag(L"check-each");
  bool blVariablePasswLen = nCharsLen != 0 &&
    (nPasswFlags & PASSW_FLAG_REMOVEWHITESPACE) &&
    (gen.CustomCharSetType == cstStandard ||
     gen.CustomCharSetType == cstStandardWithFreq) &&
    gen.CustomCharSetW32.find_first_of(WCharToW32String(L" \t")) != w32string::npos;
  std::unordered_set<SecureWString,SecureStringHashFunction> uniquePasswList;

  double dBasePasswSec = 0, dPasswSec = 0;
  int nPasswLen = 0;
  int nPasswLenWChars = 0;
  long long qPasswCnt = 0, qRetries = 0, qDuplicates = 0;
  wchar_t nullChar = '\0';

  j.key("passwords").beginArray();

  while (qPasswCnt < qNumOfPassw) {
    int nGenCharsLen = 0;
    word32* pPassw = nullptr;

    if (nCharsLen != 0) {
      if (blKeepPrevPassw) {
        nGenCharsLen = sChars.StrLen();
      }
      else {
        switch (gen.CustomCharSetType) {
        case cstStandard:
        case cstStandardWithFreq:
        default:
          nGenCharsLen = gen.GetPassword(sChars, nCharsLen, nPasswFlags);
          break;
        case cstPhonetic:
        case cstPhoneticUpperCase:
        case cstPhoneticMixedCase:
          nGenCharsLen = gen.GetPhoneticPassw(sChars, nCharsLen, nPasswFlags);
        }
        if (blFirstGen || blVariablePasswLen) {
          if ((gen.CustomCharSetType == cstStandard ||
              gen.CustomCharSetType == cstStandardWithFreq) && blEachCharOnlyOnce)
            dBasePasswSec = gen.CalcPermSetEntropy(nCharSetSize, nGenCharsLen);
          else
            dBasePasswSec = gen.CustomCharSetEntropy * nGenCharsLen;
        }
      }
      nPasswLen = nGenCharsLen;
      pPassw = sChars;
    }

    int nNetWordsLen = 0;
    if (nNumOfWords != 0) {
      nPasswLen = gen.GetPassphrase(sWords, nNumOfWords, sChars,
        nGenCharsLen, nPassphrFlags, &nNetWordsLen);
      sWords.GrowClearMark(nPasswLen);

      if (nPassphrMinLength >= 0) {
        int nBaseLen = blPassphrLenAllChars ? nPasswLen : nNetWordsLen;
        if (nBaseLen < nPassphrMinLength || nBaseLen > nPassphrMaxLength) {
          blKeepPrevPassw = true;
          qRetries++;
          continue;
        }
      }

      if (blFirstGen || blVariablePasswLen) {
        if (blEachWordOnlyOnce)
          dBasePasswSec += gen.CalcPermSetEntropy(gen.WordListSize, nNumOfWords);
        else
          dBasePasswSec += gen.WordListEntropy * nNumOfWords;
      }
      pPassw = sWords;
    }

    dPasswSec = dBasePasswSec;
    blKeepPrevPassw = false;

    int nPasswPhUsed = 0;
    w32string sInvalidSpec;
    double dFormatSec = 0;
    if (!sFormatPassw.empty()) {
      int nFormattedLen = gen.GetFormatPassw(sFormatted, sFormatPassw,
        nFormatFlags, pPassw, &nPasswPhUsed, &sInvalidSpec,
        (blFirstGen || blCheckEachPassw) ? &dFormatSec : nullptr);
      sFormatted.GrowClearMark(nFormattedLen);
      if (dFormatSec > 0) {
        if (nPasswPhUsed == PASSFORMAT_PWUSED_NOSPECIFIER)
          dPasswSec = dFormatSec;
        else
          dPasswSec += dFormatSec;
      }
      if (blFirstGen && nPasswPhUsed == PASSFORMAT_PWUSED_NOSPECIFIER) {
        sChars.Clear();
        sWords.Clear();
        nCharsLen = nNumOfWords = 0;
        dBasePasswSec = 0;
      }
      pPassw = sFormatted;
      nPasswLen = nFormattedLen;
    }

    wchar_t* pwszPassw = reinterpret_cast<wchar_t*>(pPassw);
    if (pwszPassw != nullptr) {
      W32CharToWCharInternal(pwszPassw);
      nPasswLenWChars = wcslen(pwszPassw);
      if (blFirstCharNotLC)
        pwszPassw[0] = toupper(pwszPassw[0]);
    }

    double dScriptSec = 0;
    if (pScript) {
      // TScriptingThread::Execute (Scripting.cpp:461 ff.)
      if (blScriptInit) {
        blScriptInit = false;
        pScript->InitGenerate(qNumOfPassw, static_cast<int>(a.num(L"dest", 0)),
          nInitGenFlags, static_cast<int>(a.num(L"advanced-flags", 0)),
          nInitCharsLen, nInitNumOfWords, sInitPasswFormat.c_str());
      }
      SecureWString sScriptPassw;
      pScript->CallGenerate(qPasswCnt + 1, pwszPassw, dPasswSec,
        sScriptPassw, dScriptSec);
      // Main.cpp:2590 ff.
      if (!sScriptPassw.IsStrEmpty()) {
        sFromScript.AssignStr(sScriptPassw, std::min<word32>(
            PASSWFORMAT_MAX_CHARS, sScriptPassw.StrLen()));
        pwszPassw = sFromScript;
        nPasswLen = GetNumOfUnicodeChars(sFromScript);
        nPasswLenWChars = sFromScript.StrLen();
      }
      dPasswSec = std::min(20.0 * sFromScript.StrLen(),
          std::max(0.0, dScriptSec));
    }

    if (pwszPassw == nullptr)
      pwszPassw = &nullChar;

    if (qNumOfPassw > 1 && nPasswLenWChars > 0 && blExcludeDuplicates) {
      auto ret = uniquePasswList.emplace(pwszPassw, nPasswLenWChars + 1);
      if (!ret.second) {
        qDuplicates++;
        continue;
      }
    }

    if (blFirstGen || blCheckEachPassw)
      dPasswSec = std::min<double>(dPasswSec, RandomPool::MAX_ENTROPY);

    j.beginObject();
    // (the app writes nPasswLenWChars characters; bounded here by the string)
    j.key("password").wstr(pwszPassw, std::min<size_t>(nPasswLenWChars,
      wcslen(pwszPassw)));
    j.key("len").num(nPasswLen);
    j.key("net_words_len").num(nNetWordsLen);
    j.dblWithBits("security", dPasswSec);
    if (blFirstGen || blCheckEachPassw)
      j.key("floor_entropy_bits").num(FloorEntropyBits(dPasswSec));
    if (!sFormatPassw.empty()) {
      j.key("passw_used").num(nPasswPhUsed);
      writeW32(j, "invalid_spec", sInvalidSpec);
    }
    if (pScript)
      j.dblWithBits("script_security", dScriptSec);
    j.key("retries_before").num(qRetries);
    j.key("duplicates_before").num(qDuplicates);
    j.endObject();

    qRetries = qDuplicates = 0;
    blFirstGen = false;
    qPasswCnt++;
  }
  j.endArray();
}

static void cmdLua(const Args& a, JsonWriter& j)
{
  if (!a.has(L"script"))
    throw HarnessError("missing required option --script");
  cmdGen(a, j, L"main");
}

static void cmdZxcvbn(const Args& a, JsonWriter& j)
{
  // Main.cpp:3698: ZxcvbnMatch(WStringToUtf8_s(sPassw).c_str(), nullptr, nullptr)
  SecureWString sPassw = toSWS(getWStr(a, L"passw", L"", true));
  ZxcMatch_t* pInfo = nullptr;
  double d = ZxcvbnMatch(WStringToUtf8_s(sPassw).c_str(), nullptr, &pInfo);
  j.dblWithBits("entropy", d);
  j.key("floor_entropy_bits").num(FloorEntropyBits(d));
  j.key("matches").beginArray();
  for (ZxcMatch_t* p = pInfo; p != nullptr; p = p->Next) {
    j.beginObject();
    j.key("begin").num(p->Begin);
    j.key("length").num(p->Length);
    j.key("type").num(static_cast<int>(p->Type));
    j.dblWithBits("entropy", p->Entrpy);
    j.dblWithBits("mlt_entropy", p->MltEnpy);
    j.endObject();
  }
  j.endArray();
  ZxcvbnFreeInfo(pInfo);
}

static void cmdEstimate(const Args& a, JsonWriter& j)
{
  std::wstring s = getWStr(a, L"passw", L"", true);
  double d = PasswordGenerator::EstimatePasswSecurity(s.c_str());
  j.dblWithBits("security", d);
  j.key("floor_entropy_bits").num(FloorEntropyBits(d));
}

static void cmdCharsets(const Args& a, JsonWriter& j)
{
  // only set up a generator and report the character set info
  AESCtrPRNG prng;
  prng.Seed("", 0);
  PasswordGenerator gen(&prng);
  setupGenerator(a, j, gen);
  writeGenInfo(j, gen);
  if (a.has(L"perm-set-size")) {
    j.dblWithBits("perm_set_entropy", PasswordGenerator::CalcPermSetEntropy(
      static_cast<int>(a.reqNum(L"perm-set-size")),
      static_cast<int>(a.num(L"perm-samples", 1))));
  }
  if (a.has(L"word-index")) {
    WString w = gen.GetWord(static_cast<int>(a.reqNum(L"word-index")));
    j.key("word").wstr(w.c_str(), w.Length());
  }
}

// ---------------------------------------------------------------------------
// crypt text

static const char* cryptTextResultName(int n)
{
  static const char* NAMES[] = { "OK", "CLIPBOARD", "NOTEXT", "TEXTTOOLONG",
    "OUTOFMEMORY", "TEXTCORRUPTED", "BADKEY", "DECOMPRFAILED" };
  return (n >= 0 && n <= 7) ? NAMES[n] : "?";
}

static void writeClipboard(JsonWriter& j)
{
  j.key("clipboard_set").boolean(g_capturedClipboard.blSet);
  if (g_capturedClipboard.blSet) {
    j.key("text").wstr(g_capturedClipboard.sText);
    j.key("via_ansi").boolean(g_capturedClipboard.blViaAnsi);
    if (g_capturedClipboard.blViaAnsi)
      j.key("ansi_bytes").hex(g_capturedClipboard.asAnsiBytes.data(),
        g_capturedClipboard.asAnsiBytes.size());
  }
}

static void cmdCryptTextEnc(const Args& a, JsonWriter& j)
{
  SecureWString sPassw = toSWS(getWStr(a, L"passw", L"", true));
  SecureWString sText = toSWS(getWStr(a, L"text", L"", true));
  Bytes seed = getBytes(a, L"rand-seed");

  // random source for IV and padding: AESCtrPRNG::Seed(rand-seed)
  AESCtrPRNG randGen;
  randGen.Seed(seed.data(), static_cast<word32>(seed.size()));

  g_capturedClipboard = CapturedClipboard();
  // Main.cpp:1534
  int nResult = EncryptText(&sText, sPassw.Bytes(), sPassw.StrLenBytes(), randGen);
  j.key("result").num(nResult);
  j.key("result_name").str(cryptTextResultName(nResult));
  writeClipboard(j);
}

static void cmdCryptTextDec(const Args& a, JsonWriter& j)
{
  SecureWString sPassw = toSWS(getWStr(a, L"passw", L"", true));
  SecureWString sText = toSWS(getWStr(a, L"text", L"", true));
  UINT codePage = static_cast<UINT>(a.num(L"codepage", GetACP()));
  j.key("codepage").num(codePage);

  // Main.cpp:1541-1567
  SecureAnsiString asPasswAnsi;
  {
    int nAnsiLen = WideCharToMultiByte(codePage, 0, sPassw, -1, nullptr, 0,
      nullptr, nullptr);
    asPasswAnsi.New(nAnsiLen);
    WideCharToMultiByte(codePage, 0, sPassw, -1, asPasswAnsi, nAnsiLen,
      nullptr, nullptr);
  }

  auto tryVersion = [&](int nTryVersion) {
    const word8* pTryPassw;
    int nTryPasswBytes;
    if (nTryVersion < 2) {
      pTryPassw = asPasswAnsi.Bytes();
      nTryPasswBytes = asPasswAnsi.StrLenBytes();
    }
    else {
      pTryPassw = sPassw.Bytes();
      nTryPasswBytes = sPassw.StrLenBytes();
    }
    g_capturedClipboard = CapturedClipboard();
    return DecryptText(&sText, pTryPassw, nTryPasswBytes, nTryVersion);
  };

  int nResult;
  int nTryVersion;
  j.key("attempts").beginArray();
  if (a.flag(L"old")) {
    // "PWGen <2.3.0" checkbox: versions 0, 1, 2 in turn
    nTryVersion = 0;
    do {
      nResult = tryVersion(nTryVersion);
      j.beginObject();
      j.key("version").num(nTryVersion);
      j.key("result").num(nResult);
      j.endObject();
    } while ((nResult == CRYPTTEXT_ERROR_TEXTCORRUPTED ||
        nResult == CRYPTTEXT_ERROR_BADKEY) &&
      ++nTryVersion < CRYPTTEXT_VERSION);
  }
  else {
    nTryVersion = static_cast<int>(a.num(L"version", CRYPTTEXT_VERSION));
    nResult = tryVersion(nTryVersion);
    j.beginObject();
    j.key("version").num(nTryVersion);
    j.key("result").num(nResult);
    j.endObject();
  }
  j.endArray();
  j.key("result").num(nResult);
  j.key("result_name").str(cryptTextResultName(nResult));
  if (nResult == CRYPTTEXT_OK)
    writeClipboard(j);
}

// ---------------------------------------------------------------------------
// password database

static SecureMem<word8> passwBinary(const std::wstring& sPassw, bool blUtf8)
{
  // PasswEnter.cpp:276 (GetPasswBinary)
  SecureWString s = toSWS(sPassw);
  if (s.IsStrEmpty())
    return SecureMem<word8>();
  if (blUtf8) {
    SecureAnsiString sUtf8 = WStringToUtf8_s(s);
    return SecureMem<word8>(sUtf8.Bytes(), sUtf8.StrLenBytes());
  }
  return SecureMem<word8>(s.Bytes(), s.StrLenBytes());
}

static std::wstring jStr(const JValue* v, const char* k, const std::wstring& def = L"")
{
  const JValue* x = v ? v->get(k) : nullptr;
  if (!x || x->type == JValue::Null)
    return def;
  if (x->type != JValue::String)
    throw HarnessError(std::string("spec: '") + k + "' must be a string");
  return x->s;
}

static bool jHas(const JValue* v, const char* k)
{
  const JValue* x = v ? v->get(k) : nullptr;
  return x && x->type != JValue::Null;
}

static unsigned long long jU64(const JValue* v, const char* k, unsigned long long def = 0)
{
  const JValue* x = v ? v->get(k) : nullptr;
  if (!x || x->type == JValue::Null)
    return def;
  if (x->type == JValue::Bool)
    return x->b ? 1 : 0;
  if (x->type == JValue::String)   // allow big numbers as strings
    return std::stoull(utf16ToUtf8(x->s), nullptr, 0);
  if (x->type != JValue::Number || !x->isInt)
    throw HarnessError(std::string("spec: '") + k + "' must be an integer");
  return x->isInt && x->i < 0 ? static_cast<unsigned long long>(x->i) : x->u;
}

static bool jBool(const JValue* v, const char* k, bool def)
{
  const JValue* x = v ? v->get(k) : nullptr;
  if (!x || x->type == JValue::Null)
    return def;
  if (x->type == JValue::Bool)
    return x->b;
  if (x->type == JValue::Number)
    return x->num != 0;
  throw HarnessError(std::string("spec: '") + k + "' must be a boolean");
}

// expiry date: integer (encoded) or {"y":..,"m":..,"d":..} or "YYYY-MM-DD"
static word32 jExpiry(const JValue* v, const char* k)
{
  const JValue* x = v ? v->get(k) : nullptr;
  if (!x || x->type == JValue::Null)
    return 0;
  if (x->type == JValue::Object)
    return PasswDbEntry::EncodeExpiryDate(static_cast<int>(jU64(x, "y")),
      static_cast<int>(jU64(x, "m")), static_cast<int>(jU64(x, "d")));
  if (x->type == JValue::String) {
    int y = 0, m = 0, d = 0;
    if (swscanf(x->s.c_str(), L"%d-%d-%d", &y, &m, &d) == 3)
      return PasswDbEntry::EncodeExpiryDate(y, m, d);
  }
  return static_cast<word32>(jU64(v, k));
}

static std::wstring specPath(const std::wstring& p, const std::wstring& specDir)
{
  if (p.empty())
    return p;
  std::wstring q = p;
  for (auto& c : q)
    if (c == '/')
      c = '\\';
  bool blAbs = (q.size() >= 1 && q[0] == '\\') || (q.size() >= 2 && q[1] == ':');
  if (!blAbs)
    q = specDir + q;
  return winPath(q);
}

static void cmdDbCreate(const Args& a, JsonWriter& j)
{
  std::wstring sSpecFile = winPath(a.req(L"spec"));
  std::wstring sSpecDir = toWS(sSpecFile).IsEmpty() ? L"" :
    std::wstring(ExtractFilePath(toWS(sSpecFile)).c_str());
  Bytes specText = readFile(sSpecFile);
  JPtr spec = JsonParser(std::string(specText.begin(), specText.end())).parse();
  if (spec->type != JValue::Object)
    throw HarnessError("spec must be a JSON object");
  std::wstring sOut = winPath(a.req(L"out"));

  // deterministic RandomPool (salt, IV, padding, recovery key) unless the
  // global --det-rand-seed has already been given
  static AESCtrPRNG s_specRand;
  if (jHas(spec.get(), "rand_seed_hex") && !DetRand_IsEnabled()) {
    Bytes seed = fromHex(utf16ToUtf8(jStr(spec.get(), "rand_seed_hex")));
    s_specRand.Seed(seed.data(), static_cast<word32>(seed.size()));
    DetRand_Enable(&s_specRand);
    word32 fseed[3] = { 0x12345678, 0x9abcdef0, 0x0fedcba9 };
    g_fastRandGen.Seed(fseed, sizeof(fseed));
  }
  j.key("deterministic").boolean(DetRand_IsEnabled());

  PasswDatabase db;
  bool blUtf8 = jBool(spec.get(), "passw_utf8", true);
  db.IsPasswUtf8 = blUtf8;
  db.CipherType = static_cast<word8>(jU64(spec.get(), "cipher", 0));
  db.KdfIterations = static_cast<word32>(jU64(spec.get(), "kdf_iterations",
    PasswDatabase::KEY_HASH_ITERATIONS));

  std::wstring sKeyFile = specPath(jStr(spec.get(), "keyfile"), sSpecDir);
  SecureMem<word8> key = PasswDatabase::CombineKeySources(
    passwBinary(jStr(spec.get(), "password"), blUtf8), toWS(sKeyFile));
  db.New(key);

  db.DefaultUserName = toSWS(jStr(spec.get(), "default_user_name"));
  db.PasswFormatSeq = toSWS(jStr(spec.get(), "passw_format_seq"));
  db.Description = toSWS(jStr(spec.get(), "description"));
  db.DefaultPasswExpiryDays = static_cast<word32>(jU64(spec.get(), "default_passw_expiry_days"));
  db.DefaultMaxPasswHistorySize = static_cast<word32>(
    jU64(spec.get(), "default_max_passw_history_size"));
  db.MasterPasswExpiryDate = jExpiry(spec.get(), "master_passw_expiry_date");
  db.Compressed = jBool(spec.get(), "compressed", false);
  db.CompressionLevel = static_cast<int>(jU64(spec.get(), "compression_level", 0));

  const JValue* entries = spec->get("entries");
  if (entries && entries->type == JValue::Array) {
    for (const auto& ePtr : entries->arr) {
      const JValue* e = ePtr.get();
      PasswDbEntry* pEntry = db.NewDbEntry();
      // NewDbEntry() stamps the current local time; the spec decides instead
      pEntry->CreationTime = u64ToFileTime(jU64(e, "creation_time"));
      pEntry->ModificationTime = u64ToFileTime(jU64(e, "modification_time"));
      pEntry->PasswChangeTime = u64ToFileTime(jU64(e, "passw_change_time"));

      static const char* FIELD_KEYS[] = { "title", "user_name", "password",
        "url", "keyword", "notes" };
      for (int i = 0; i < 6; i++) {
        if (i == PasswDbEntry::PASSWORD) {
          db.SetDbEntryPassw(*pEntry, toSWS(jStr(e, "password")));
        }
        else if (i != PasswDbEntry::USERNAME || jHas(e, FIELD_KEYS[i]))
          pEntry->Strings[i] = toSWS(jStr(e, FIELD_KEYS[i]));
      }

      const JValue* kv = e->get("key_values");
      if (kv && kv->type == JValue::Array) {
        PasswDbEntry::KeyValueList list;
        for (const auto& p : kv->arr) {
          if (p->type != JValue::Array || p->arr.size() != 2)
            throw HarnessError("spec: key_values items must be [key, value]");
          list.emplace_back(toSWS(p->arr[0]->s), toSWS(p->arr[1]->s));
        }
        pEntry->SetKeyValueList(list);
      }

      const JValue* tags = e->get("tags");
      if (tags && tags->type == JValue::Array) {
        // tags are SecureWStrings of size len+1 (as in PasswManager.cpp:2190)
        std::set<SecureWString> tagSet;
        for (const auto& t : tags->arr)
          tagSet.insert(toSWS(t->s));
        pEntry->SetTagList(tagSet);
      }

      pEntry->PasswExpiryDate = jExpiry(e, "passw_expiry_date");

      const JValue* h = e->get("history");
      if (h && h->type == JValue::Object) {
        auto& hist = pEntry->GetPasswHistory();
        hist.SetActive(true);
        hist.SetMaxSize(static_cast<word32>(jU64(h, "max_size", 1)));
        hist.ClearHistory();
        const JValue* he = h->get("entries");
        if (he && he->type == JValue::Array) {
          // listed newest first, i.e. in file/display order
          for (const auto& x : he->arr)
            hist.AddEntry({ u64ToFileTime(jU64(x.get(), "time")),
              toSWS(jStr(x.get(), "password")) }, false);
        }
        hist.SetActive(jBool(h, "active", true));
      }
    }
  }

  if (jHas(spec.get(), "recovery_password") || jHas(spec.get(), "recovery_keyfile")) {
    std::wstring sRecKeyFile = specPath(jStr(spec.get(), "recovery_keyfile"), sSpecDir);
    auto recoveryKey = PasswDatabase::CombineKeySources(
      passwBinary(jStr(spec.get(), "recovery_password"), blUtf8), toWS(sRecKeyFile));
    db.SetRecoveryKey(key, recoveryKey);
  }

  db.Save(toWS(sOut));
  db.Close();

  Bytes file = readFile(sOut);
  j.key("out").wstr(sOut);
  j.key("file_size").num(static_cast<long long>(file.size()));
  j.key("file_sha256").hex(sha256Of(file.data(), file.size()));
  if (a.flag(L"digest") == false && file.size() <= 1 << 20)
    j.key("file_hex").hex(file);
}

static void writeFileLayout(JsonWriter& j, const Bytes& f)
{
  // plaintext file header (PasswDatabase.cpp:65) and the v1.8 layout
  j.key("file_size").num(static_cast<long long>(f.size()));
  j.key("file_sha256").hex(sha256Of(f.data(), f.size()));
  if (f.size() < 18)
    return;
  auto u32 = [&](size_t o) {
    return static_cast<unsigned long long>(f[o]) | (f[o + 1] << 8) |
      (f[o + 2] << 16) | (static_cast<unsigned long long>(f[o + 3]) << 24);
  };
  j.key("header").beginObject();
  j.key("magic").unum(u32(0));
  j.key("header_size").num(f[4]);
  j.key("version").num(f[5] | (f[6] << 8));
  j.key("flags").unum(u32(7));
  j.key("cipher_type").num(f[11]);
  j.key("hash_type").num(f[12]);
  j.key("kdf_type").num(f[13]);
  j.key("kdf_iterations").unum(u32(14));
  j.endObject();

  size_t pos = f[4];
  int nVersion = f[5] | (f[6] << 8);
  unsigned long long flags = u32(7);
  if (nVersion != 0x108 || pos > f.size())
    return;
  size_t ivLen = (f[11] == 1) ? 8 : 16;
  size_t hmacLen = (f[12] == 1) ? 64 : 32;
  j.key("layout").beginObject();
  if (flags & 1) {
    if (pos + 128 > f.size()) { j.endObject(); return; }
    j.key("recovery_block").hex(f.data() + pos, 128);
    pos += 128;
  }
  if (pos + 32 + ivLen + hmacLen <= f.size()) {
    j.key("salt").hex(f.data() + pos, 32);
    pos += 32;
    j.key("iv").hex(f.data() + pos, ivLen);
    pos += ivLen;
    j.key("ciphertext_len").num(static_cast<long long>(f.size() - pos - hmacLen));
    j.key("hmac").hex(f.data() + f.size() - hmacLen, hmacLen);
  }
  j.endObject();
}

static void cmdDbDump(const Args& a, JsonWriter& j)
{
  std::wstring sIn = winPath(a.req(L"in"));
  Bytes file = readFile(sIn);
  writeFileLayout(j, file);
  if (a.flag(L"header-only"))
    return;

  std::wstring sKeyFile = a.has(L"keyfile") ? winPath(a.str(L"keyfile")) : L"";
  std::wstring sPassw = getWStr(a, L"passw");

  // PasswManager.cpp:728-740
  PasswDatabase db;
  db.Open(SecureMem<word8>(), toWS(sIn), true);
  bool blUtf8 = db.IsPasswUtf8;
  if (a.flag(L"utf16-passw"))
    blUtf8 = false;
  SecureMem<word8> key = PasswDatabase::CombineKeySources(
    passwBinary(sPassw, blUtf8), toWS(sKeyFile));
  db.Open(key, toWS(sIn));

  j.key("db").beginObject();
  j.key("version").num(db.Version);
  j.key("cipher_type").num(db.CipherType);
  j.key("kdf_iterations").unum(db.KdfIterations);
  j.key("has_recovery_key").boolean(db.HasRecoveryKey);
  j.key("compressed").boolean(db.Compressed);
  j.key("compression_level").num(db.CompressionLevel);
  j.key("passw_utf8").boolean(db.IsPasswUtf8);
  j.key("default_user_name").wstr(fromSWS(db.DefaultUserName));
  j.key("passw_format_seq").wstr(fromSWS(db.PasswFormatSeq));
  j.key("description").wstr(fromSWS(db.Description));
  j.key("default_passw_expiry_days").unum(db.DefaultPasswExpiryDays);
  j.key("default_max_passw_history_size").unum(db.DefaultMaxPasswHistorySize);
  j.key("master_passw_expiry_date").unum(db.MasterPasswExpiryDate);
  j.key("size").unum(db.Size);
  j.key("check_master_key").boolean(db.CheckMasterKey(key));
  j.endObject();

  j.key("entries").beginArray();
  for (const auto& pEntry : db) {
    j.beginObject();
    j.key("index").unum(pEntry->GetIndex());
    j.key("id").unum(pEntry->GetId());
    j.key("title").wstr(fromSWS(pEntry->Strings[PasswDbEntry::TITLE]));
    j.key("user_name").wstr(fromSWS(pEntry->Strings[PasswDbEntry::USERNAME]));
    j.key("password").wstr(fromSWS(db.GetDbEntryPassw(*pEntry)));
    j.key("url").wstr(fromSWS(pEntry->Strings[PasswDbEntry::URL]));
    j.key("keyword").wstr(fromSWS(pEntry->Strings[PasswDbEntry::KEYWORD]));
    j.key("notes").wstr(fromSWS(pEntry->Strings[PasswDbEntry::NOTES]));
    j.key("key_value_string").wstr(fromSWS(pEntry->Strings[PasswDbEntry::KEYVALUELIST]));
    j.key("key_values").beginArray();
    for (const auto& kv : pEntry->GetKeyValueList()) {
      j.beginArray();
      j.wstr(fromSWS(kv.first));
      j.wstr(fromSWS(kv.second));
      j.endArray();
    }
    j.endArray();
    j.key("tags_string").wstr(fromSWS(pEntry->Strings[PasswDbEntry::TAGS]));
    j.key("tags").beginArray();
    for (const auto& t : pEntry->GetTagList())
      j.wstr(fromSWS(t));
    j.endArray();
    j.key("creation_time").unum(fileTimeToU64(pEntry->CreationTime));
    j.key("modification_time").unum(fileTimeToU64(pEntry->ModificationTime));
    j.key("passw_change_time").unum(fileTimeToU64(pEntry->PasswChangeTime));
    j.key("passw_expiry_date").unum(pEntry->PasswExpiryDate);
    const auto& hist = pEntry->GetPasswHistory();
    j.key("history").beginObject();
    j.key("active").boolean(hist.GetActive());
    j.key("max_size").unum(hist.GetMaxSize());
    j.key("entries").beginArray();
    for (const auto& he : hist) {
      j.beginObject();
      j.key("time").unum(fileTimeToU64(he.first));
      j.key("password").wstr(fromSWS(he.second));
      j.endObject();
    }
    j.endArray();
    j.endObject();
    // locale-dependent strings (GetDateFormatEx/GetTimeFormatEx under wine)
    j.key("strings_locale").beginObject();
    j.key("creation_time").wstr(fromSWS(pEntry->CreationTimeString));
    j.key("modification_time").wstr(fromSWS(pEntry->ModificationTimeString));
    j.key("passw_change_time").wstr(fromSWS(pEntry->PasswChangeTimeString));
    j.key("passw_expiry_date").wstr(fromSWS(pEntry->PasswExpiryDateString));
    j.endObject();
    j.endObject();
  }
  j.endArray();
}

static void cmdKeyfile(const Args& a, JsonWriter& j)
{
  std::wstring sIn = winPath(a.req(L"in"));
  auto key = PasswDatabase::GetKeyFromKeyFile(toWS(sIn));
  j.key("key").hex(key.Data(), key.Size());
}

static void cmdKeyfileCreate(const Args& a, JsonWriter& j)
{
  std::wstring sOut = winPath(a.req(L"out"));
  PasswDatabase::CreateKeyFile(toWS(sOut));
  Bytes f = readFile(sOut);
  j.key("out").wstr(sOut);
  j.key("content").str(std::string(f.begin(), f.end()));
  auto key = PasswDatabase::GetKeyFromKeyFile(toWS(sOut));
  j.key("key").hex(key.Data(), key.Size());
}

// ---------------------------------------------------------------------------
// trigram file

static void cmdTrigramCreate(const Args& a, JsonWriter& j)
{
  std::wstring sSrc = winPath(a.req(L"src"));
  std::wstring sDst = winPath(a.req(L"dst"));
  auto result = PasswordGenerator::CreateTrigramFile(toWS(sSrc), toWS(sDst));
  j.key("sigma").unum(result.first);
  j.dblWithBits("entropy", result.second);
  Bytes f = readFile(sDst);
  j.key("file_size").num(static_cast<long long>(f.size()));
  j.key("file_sha256").hex(sha256Of(f.data(), f.size()));

  // load it back like the generator does
  AESCtrPRNG prng;
  prng.Seed("", 0);
  PasswordGenerator gen(&prng);
  j.key("load_result").num(gen.LoadTrigramFile(toWS(sDst)));
  j.dblWithBits("phonetic_entropy_after_load", gen.PhoneticEntropy);
}

// ---------------------------------------------------------------------------
// MP password generator (MPPasswGen.cpp:286-470, GUI code re-implemented with
// the original primitives)

static void cmdMppg(const Args& a, JsonWriter& j)
{
  SecureWString sPassw = toSWS(getWStr(a, L"passw", L"", true));
  WString sParam = toWS(getWStr(a, L"param"));
  int nCharSetIdx = static_cast<int>(a.num(L"charset-index", 0));
  int nPasswLen = static_cast<int>(a.num(L"length", 16));
  bool blHashapass = a.flag(L"hashapass");
  if (nCharSetIdx < 0 || nCharSetIdx >= MPPG_CHARSETS_NUM)
    throw HarnessError("--charset-index must be 0..5");

  // EnterPasswBtnClick (MPPasswGen.cpp:286 ff.)
  SecureMem<word8> m_key;
  SecureMem<word8> tempKey(AESCtrPRNG::KEY_SIZE);
  if (blHashapass) {
    m_key.New(sPassw.StrLen());
    for (word32 i = 0; i < m_key.Size(); i++)     // unicodeToLSByte()
      m_key[i] = static_cast<word8>(static_cast<char>(sPassw[i]));
  }
  sha256_hmac(sPassw.Bytes(), sPassw.StrLenBytes(),
    reinterpret_cast<const word8*>(MPPG_KEYGEN_SALTSTR),
    sizeof(MPPG_KEYGEN_SALTSTR) - 1, tempKey.Data(), 0);
  if (!blHashapass)
    m_key = tempKey;
  j.key("key").hex(m_key.Data(), m_key.Size());

  // password hash shown next to the status ("Show password hash")
  {
    SecureMem<word8> hash(32);
    sha256(tempKey, tempKey.Size(), hash, 0);
    j.key("passw_hash").beginObject();
    for (int nHashType = 0; nHashType < 4; nHashType++) {
      word32 lHashValue = 0;
      for (int i = 0; i < (nHashType == 2 ? 2 : 4); i++)
        lHashValue = (lHashValue << 8) | hash[i];
      char buf[16];
      switch (nHashType) {
      case 0: snprintf(buf, sizeof(buf), "%.2u", lHashValue % 100); break;
      case 1: snprintf(buf, sizeof(buf), "%.4u", lHashValue % 10000); break;
      case 2: snprintf(buf, sizeof(buf), "%.4x", lHashValue & 0xffff); break;
      case 3: snprintf(buf, sizeof(buf), "%.8x", lHashValue); break;
      }
      static const char* NAMES[] = { "dec99", "dec9999", "hex16", "hex32" };
      j.key(NAMES[nHashType]).str(buf);
    }
    j.endObject();
  }

  // GenerateBtnClick (MPPasswGen.cpp:388 ff.)
  double dPasswBits;
  std::wstring sResult;
  int nParamLen = sParam.Length();
  if (blHashapass) {
    AnsiString asParam;
    if (nParamLen != 0) {
      asParam.SetLength(nParamLen);
      for (int i = 0; i < nParamLen; i++)       // unicodeToLSByte()
        asParam.c_str()[i] = static_cast<char>(sParam.c_str()[i]);
    }
    SecureMem<word8> passwBytes(20);
    sha1_hmac(m_key.Data(), m_key.Size(),
      reinterpret_cast<const word8*>(asParam.c_str()), asParam.Length(),
      passwBytes.Data());
    nPasswLen = 8;
    SecureAnsiString asPassw(nPasswLen + 1);
    size_t outputLen = asPassw.Size();
    base64_encode(reinterpret_cast<word8*>(asPassw.Data()),
      &outputLen, passwBytes.Data(), 6, 0);
    // upstream copies outputLen (=8) chars into a SecureWString(8) without a
    // terminating zero (asciiToUnicode) and passes it to SetWindowText, i.e.
    // it relies on whatever follows in memory; the intended 8 chars are:
    for (size_t i = 0; i < outputLen; i++)
      sResult.push_back(static_cast<wchar_t>(asPassw[static_cast<word32>(i)]));
    j.key("hashapass_output_len").num(static_cast<long long>(outputLen));
    dPasswBits = nPasswLen * 6;
  }
  else {
    bool addLength = a.flag(L"add-length");
    word32 lParamBytes = sParam.Length() * sizeof(wchar_t);
    SecureMem<word8> paramData(lParamBytes + (addLength ? 3 : 0));
    if (lParamBytes)
      memcpy(paramData, reinterpret_cast<word8*>(sParam.c_str()), lParamBytes);
    if (addLength) {
      paramData[lParamBytes] = 0;
      paramData[lParamBytes + 1] = nPasswLen & 0xff;
      paramData[lParamBytes + 2] = (nPasswLen >> 8) & 0xff;
    }
    j.key("param_data").hex(paramData.Data(), paramData.Size());

    AESCtrPRNG randGen;
    randGen.SeedWithKey(m_key, m_key.Size(), paramData, paramData.Size());

    const char* pszCharSet = MPPG_CHARSETS[nCharSetIdx];
    int nCharSetSize = strlen(pszCharSet);
    SecureWString sPassw2(nPasswLen + 1);
    for (int i = 0; i < nPasswLen; i++)
      sPassw2[i] = pszCharSet[randGen.GetNumRange(nCharSetSize)];
    sPassw2[nPasswLen] = '\0';
    sResult = sPassw2.c_str();
    dPasswBits = Log2(static_cast<double>(nCharSetSize)) * nPasswLen;
  }
  j.key("password").wstr(sResult);
  j.dblWithBits("bits", dPasswBits);
  j.key("length").num(nPasswLen);
}

// ---------------------------------------------------------------------------

using CmdFn = void (*)(const Args&, JsonWriter&);

static std::string demangledTypeName(const std::type_info& ti)
{
  int status = 0;
  char* p = abi::__cxa_demangle(ti.name(), nullptr, nullptr, &status);
  std::string s = (status == 0 && p) ? p : ti.name();
  free(p);
  return s;
}

static void usage()
{
  fputs(
    "usage: refharness [--det-rand-seed HEX] [--file-encoding N] <command> [options]\n"
    "commands: prng-aesctr pbkdf2 hmac-sha256 hmac-sha512 hmac-sha1 sha256 sha512\n"
    "          sha1 chacha gen charsets estimate crypt-text-enc crypt-text-dec\n"
    "          db-create db-dump keyfile keyfile-create trigram-create mppg\n"
    "          zxcvbn lua\n"
    "see tools/refharness/README.md\n", stderr);
}

int wmain(int argc, wchar_t** argv)
{
  _setmode(_fileno(stdout), _O_BINARY);
  _setmode(_fileno(stderr), _O_BINARY);

  // like the original: directory of the executable with trailing '\'
  {
    wchar_t buf[MAX_PATH];
    DWORD n = GetModuleFileNameW(nullptr, buf, MAX_PATH);
    g_sExePath = ExtractFilePath(WString(buf, static_cast<int>(n)));
  }

  JsonWriter j;
  int nExit = 0;
  std::string sCmd;
  j.beginObject();
  try {
    // global options precede the command
    int i = 1;
    std::wstring sDetSeed;
    for (; i < argc; i++) {
      std::wstring arg = argv[i];
      if (arg == L"--det-rand-seed" && i + 1 < argc)
        sDetSeed = argv[++i];
      else if (arg == L"--file-encoding" && i + 1 < argc)
        g_config.FileEncoding = static_cast<CharacterEncoding>(_wtoi(argv[++i]));
      else
        break;
    }
    if (i >= argc) {
      usage();
      return 2;
    }
    sCmd = utf16ToUtf8(argv[i]);
    Args a(argc, argv, i + 1, FLAG_OPTS);
    if (!a.positional().empty())
      throw HarnessError("unexpected argument " + utf16ToUtf8(a.positional()[0]));

    static AESCtrPRNG s_detRand;
    if (!sDetSeed.empty()) {
      Bytes seed = fromHex(utf16ToUtf8(sDetSeed));
      s_detRand.Seed(seed.data(), static_cast<word32>(seed.size()));
      DetRand_Enable(&s_detRand);
      word32 fseed[3] = { 0x12345678, 0x9abcdef0, 0x0fedcba9 };
      g_fastRandGen.Seed(fseed, sizeof(fseed));
    }

    static const std::map<std::string, CmdFn> CMDS = {
      { "prng-aesctr", cmdPrngAesCtr },
      { "pbkdf2", cmdPbkdf2 },
      { "hmac-sha256", [](const Args& a, JsonWriter& j) { cmdHmac(a, j, 256); } },
      { "hmac-sha512", [](const Args& a, JsonWriter& j) { cmdHmac(a, j, 512); } },
      { "hmac-sha1", [](const Args& a, JsonWriter& j) { cmdHmac(a, j, 160); } },
      { "sha256", [](const Args& a, JsonWriter& j) { cmdHash(a, j, 256); } },
      { "sha512", [](const Args& a, JsonWriter& j) { cmdHash(a, j, 512); } },
      { "sha1", [](const Args& a, JsonWriter& j) { cmdHash(a, j, 160); } },
      { "chacha", cmdChaCha },
      { "gen", [](const Args& a, JsonWriter& j) { cmdGen(a, j, nullptr); } },
      { "charsets", cmdCharsets },
      { "estimate", cmdEstimate },
      { "crypt-text-enc", cmdCryptTextEnc },
      { "crypt-text-dec", cmdCryptTextDec },
      { "db-create", cmdDbCreate },
      { "db-dump", cmdDbDump },
      { "keyfile", cmdKeyfile },
      { "keyfile-create", cmdKeyfileCreate },
      { "trigram-create", cmdTrigramCreate },
      { "mppg", cmdMppg },
      { "zxcvbn", cmdZxcvbn },
      { "lua", cmdLua },
    };
    auto it = CMDS.find(sCmd);
    if (it == CMDS.end()) {
      usage();
      throw HarnessError("unknown command '" + sCmd + "'");
    }
    j.key("ok").boolean(true);
    j.key("cmd").str(sCmd);
    j.key("acp").num(GetACP());
    it->second(a, j);

    if (DetRand_IsEnabled()) {
      j.key("randpool_log").beginArray();
      for (const auto& e : DetRand_Log())
        j.hex(e.data(), e.size());
      j.endArray();
    }
  }
  catch (Exception& e) {
    JsonWriter err;
    err.beginObject();
    err.key("ok").boolean(false);
    err.key("cmd").str(sCmd);
    err.key("exception").str(demangledTypeName(typeid(e)));
    err.key("error").wstr(e.Message.c_str(), e.Message.Length());
    err.endObject();
    fwrite(err.result().data(), 1, err.result().size(), stdout);
    fputc('\n', stdout);
    return 1;
  }
  catch (std::exception& e) {
    JsonWriter err;
    err.beginObject();
    err.key("ok").boolean(false);
    err.key("cmd").str(sCmd);
    err.key("exception").str(dynamic_cast<HarnessError*>(&e) ? "HarnessError" :
      "std::exception");
    err.key("error").str(e.what());
    err.endObject();
    fwrite(err.result().data(), 1, err.result().size(), stdout);
    fputc('\n', stdout);
    return 1;
  }
  j.endObject();
  fwrite(j.result().data(), 1, j.result().size(), stdout);
  fputc('\n', stdout);
  return nExit;
}
