// Implementation of the VCL/RTL shim (see vclshim.h)
#include "vclshim.h"

#include <climits>
#include <cwctype>

namespace pt_finally {
std::exception_ptr& slot()
{
  static thread_local std::exception_ptr s;
  return s;
}
}

// ---------------------------------------------------------------------------
static std::wstring acpToWide(const char* p, int len)
{
  if (p == nullptr || len == 0)
    return std::wstring();
  int n = MultiByteToWideChar(CP_ACP, 0, p, len, nullptr, 0);
  if (n <= 0)
    return std::wstring();
  std::wstring w(n, L'\0');
  MultiByteToWideChar(CP_ACP, 0, p, len, &w[0], n);
  if (len < 0 && !w.empty() && w.back() == L'\0')
    w.pop_back();
  return w;
}

static std::string wideToAcp(const wchar_t* p, int len)
{
  if (p == nullptr || len == 0)
    return std::string();
  int n = WideCharToMultiByte(CP_ACP, 0, p, len, nullptr, 0, nullptr, nullptr);
  if (n <= 0)
    return std::string();
  std::string s(n, '\0');
  WideCharToMultiByte(CP_ACP, 0, p, len, &s[0], n, nullptr, nullptr);
  if (len < 0 && !s.empty() && s.back() == '\0')
    s.pop_back();
  return s;
}

// ---------------------------------------------------------------------------
UnicodeString::UnicodeString(const char* p)
  : m_s(acpToWide(p, -1))
{
}

UnicodeString::UnicodeString(const char* p, int len)
  : m_s(acpToWide(p, len > 0 ? len : 0))
{
}

UnicodeString::UnicodeString(const AnsiString& s)
  : m_s(acpToWide(s.c_str(), s.Length()))
{
}

UnicodeString::UnicodeString(char c)
  : m_s(acpToWide(&c, 1))
{
}

void UnicodeString::CheckIndex(int idx) const
{
  if (idx < 1 || idx > Length())
    throw ERangeError("Range check error");
}

UnicodeString& UnicodeString::SetLength(int n)
{
  m_s.resize(n > 0 ? n : 0, L'\0');
  return *this;
}

// System.Delete(S, Index, Count)
UnicodeString& UnicodeString::Delete(int index, int count)
{
  const int len = Length();
  if (index >= 1 && index <= len && count > 0) {
    count = std::min(count, len - index + 1);
    m_s.erase(index - 1, count);
  }
  return *this;
}

// System.Insert(Source, S, Index)
UnicodeString& UnicodeString::Insert(const UnicodeString& s, int index)
{
  if (s.IsEmpty())
    return *this;
  const int len = Length();
  if (index < 1)
    index = 1;
  if (index > len + 1)
    index = len + 1;
  m_s.insert(index - 1, s.m_s);
  return *this;
}

// System.Pos(SubStr, S)
int UnicodeString::Pos(const UnicodeString& sub) const
{
  if (sub.IsEmpty() || IsEmpty())
    return 0;
  auto p = m_s.find(sub.m_s);
  return (p == std::wstring::npos) ? 0 : static_cast<int>(p) + 1;
}

// System.Copy(S, Index, Count)
UnicodeString UnicodeString::SubString(int index, int count) const
{
  const int len = Length();
  int start;
  if (index < 1)
    start = 0;
  else {
    start = index - 1;
    if (start > len)
      start = len;
  }
  if (count < 0)
    count = 0;
  else if (count > len - start)
    count = len - start;
  return UnicodeString(m_s.substr(start, count));
}

UnicodeString UnicodeString::Trim() const { return ::Trim(*this); }
UnicodeString UnicodeString::TrimLeft() const { return ::TrimLeft(*this); }
UnicodeString UnicodeString::TrimRight() const { return ::TrimRight(*this); }
UnicodeString UnicodeString::LowerCase() const { return ::AnsiLowerCase(*this); }
UnicodeString UnicodeString::UpperCase() const { return ::AnsiUpperCase(*this); }
int UnicodeString::ToInt() const { return ::StrToInt(*this); }
int UnicodeString::ToIntDef(int def) const { return ::StrToIntDef(*this, def); }
int UnicodeString::CompareIC(const UnicodeString& o) const { return ::CompareText(*this, o); }

// ---------------------------------------------------------------------------
AnsiString::AnsiString(const UnicodeString& s)
  : m_s(wideToAcp(s.c_str(), s.Length()))
{
}

AnsiString::AnsiString(const wchar_t* p)
  : m_s(wideToAcp(p, -1))
{
}

void AnsiString::CheckIndex(int idx) const
{
  if (idx < 1 || idx > Length())
    throw ERangeError("Range check error");
}

AnsiString AnsiString::SubString(int index, int count) const
{
  const int len = Length();
  int start = (index < 1) ? 0 : std::min(index - 1, len);
  if (count < 0)
    count = 0;
  else if (count > len - start)
    count = len - start;
  return AnsiString(m_s.substr(start, count));
}

AnsiString AnsiString::Trim() const
{
  int l = Length(), i = 0;
  while (i < l && static_cast<unsigned char>(m_s[i]) <= ' ') i++;
  if (i == l)
    return AnsiString();
  while (static_cast<unsigned char>(m_s[l - 1]) <= ' ') l--;
  return AnsiString(m_s.substr(i, l - i));
}

// ---------------------------------------------------------------------------
void OutOfMemoryError()
{
  throw EOutOfMemory("Out of memory");
}

// ---------------------------------------------------------------------------
void TStream::ReadBuffer(void* pBuf, int nCount)
{
  if (nCount != 0 && Read(pBuf, nCount) != nCount)
    throw EReadError("Stream read error");
}

void TStream::WriteBuffer(const void* pBuf, int nCount)
{
  if (nCount != 0 && Write(pBuf, nCount) != nCount)
    throw EWriteError("Stream write error");
}

__int64 TStream::GetSize()
{
  __int64 pos = Seek(0, soFromCurrent);
  __int64 size = Seek(0, soFromEnd);
  Seek(pos, soFromBeginning);
  return size;
}

int THandleStream::Read(void* pBuf, int nCount)
{
  DWORD dwRead = 0;
  if (nCount <= 0)
    return 0;
  if (!ReadFile(m_hFile, pBuf, static_cast<DWORD>(nCount), &dwRead, nullptr))
    return 0;
  return static_cast<int>(dwRead);
}

int THandleStream::Write(const void* pBuf, int nCount)
{
  DWORD dwWritten = 0;
  if (nCount <= 0)
    return 0;
  if (!WriteFile(m_hFile, pBuf, static_cast<DWORD>(nCount), &dwWritten, nullptr))
    return 0;
  return static_cast<int>(dwWritten);
}

__int64 THandleStream::Seek(__int64 offset, int origin)
{
  LARGE_INTEGER li, newPos;
  li.QuadPart = offset;
  DWORD dwMethod = (origin == soFromBeginning) ? FILE_BEGIN :
    (origin == soFromCurrent) ? FILE_CURRENT : FILE_END;
  if (!SetFilePointerEx(m_hFile, li, &newPos, dwMethod))
    return -1;
  return newPos.QuadPart;
}

void THandleStream::SetSize(__int64 v)
{
  Seek(v, soFromBeginning);
  if (!SetEndOfFile(m_hFile))
    throw EOSError(SysErrorMessage(GetLastError()));
}

// System.Classes.TFileStream.Create / System.SysUtils.FileOpen/FileCreate
TFileStream::TFileStream(const UnicodeString& sFileName, Word wMode)
  : THandleStream(INVALID_HANDLE_VALUE), m_sFileName(sFileName)
{
  static const DWORD ACCESS_MODE[3] = {
    GENERIC_READ, GENERIC_WRITE, GENERIC_READ | GENERIC_WRITE };
  static const DWORD SHARE_MODE[5] = {
    0, 0, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE };

  if ((wMode & fmCreate) == fmCreate) {
    int nShare = (wMode & 0xF0) >> 4;
    m_hFile = CreateFileW(sFileName.c_str(), GENERIC_READ | GENERIC_WRITE,
      (nShare <= 4) ? SHARE_MODE[nShare] : 0, nullptr, CREATE_ALWAYS,
      FILE_ATTRIBUTE_NORMAL, nullptr);
    if (m_hFile == INVALID_HANDLE_VALUE)
      throw EFCreateError("Cannot create file \"" + ExpandFileName(sFileName) +
        "\". " + SysErrorMessage(GetLastError()));
  }
  else {
    int nAccess = wMode & 3, nShare = (wMode & 0xF0) >> 4;
    if (nAccess <= 2 && nShare <= 4)
      m_hFile = CreateFileW(sFileName.c_str(), ACCESS_MODE[nAccess],
        SHARE_MODE[nShare], nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (m_hFile == INVALID_HANDLE_VALUE)
      throw EFOpenError("Cannot open file \"" + ExpandFileName(sFileName) +
        "\". " + SysErrorMessage(GetLastError()));
  }
}

TFileStream::~TFileStream()
{
  if (m_hFile != INVALID_HANDLE_VALUE)
    CloseHandle(m_hFile);
}

int TMemoryStream::Read(void* pBuf, int nCount)
{
  __int64 avail = static_cast<__int64>(m_buf.size()) - m_pos;
  if (nCount <= 0 || avail <= 0)
    return 0;
  int n = static_cast<int>(std::min<__int64>(avail, nCount));
  memcpy(pBuf, m_buf.data() + m_pos, n);
  m_pos += n;
  return n;
}

int TMemoryStream::Write(const void* pBuf, int nCount)
{
  if (nCount <= 0)
    return 0;
  if (m_pos + nCount > static_cast<__int64>(m_buf.size()))
    m_buf.resize(static_cast<size_t>(m_pos + nCount));
  memcpy(m_buf.data() + m_pos, pBuf, nCount);
  m_pos += nCount;
  return nCount;
}

__int64 TMemoryStream::Seek(__int64 offset, int origin)
{
  if (origin == soFromBeginning)
    m_pos = offset;
  else if (origin == soFromCurrent)
    m_pos += offset;
  else
    m_pos = static_cast<__int64>(m_buf.size()) + offset;
  return m_pos;
}

// ---------------------------------------------------------------------------
UnicodeString IntToStr(int v) { return UnicodeString(v); }
UnicodeString IntToStr(__int64 v) { return UnicodeString(v); }
UnicodeString UIntToStr(unsigned v) { return UnicodeString(v); }

static bool parseInt(const UnicodeString& s, int& v)
{
  const wchar_t* p = s.c_str();
  while (*p == ' ') p++;
  bool neg = false;
  if (*p == '-' || *p == '+') {
    neg = *p == '-';
    p++;
  }
  int base = 10;
  if (*p == '$') {
    base = 16;
    p++;
  }
  else if (p[0] == '0' && (p[1] == 'x' || p[1] == 'X')) {
    base = 16;
    p += 2;
  }
  if (*p == '\0')
    return false;
  long long r = 0;
  for (; *p != '\0'; p++) {
    int d;
    if (*p >= '0' && *p <= '9') d = *p - '0';
    else if (base == 16 && *p >= 'a' && *p <= 'f') d = *p - 'a' + 10;
    else if (base == 16 && *p >= 'A' && *p <= 'F') d = *p - 'A' + 10;
    else return false;
    r = r * base + d;
    if (r > 0xFFFFFFFFLL)
      return false;
  }
  if (neg)
    r = -r;
  if (base == 10 && (r > INT_MAX || r < INT_MIN))
    return false;
  v = static_cast<int>(r);
  return true;
}

int StrToInt(const UnicodeString& s)
{
  int v;
  if (!parseInt(s, v))
    throw EConvertError("'" + s + "' is not a valid integer value");
  return v;
}

int StrToIntDef(const UnicodeString& s, int def)
{
  int v;
  return parseInt(s, v) ? v : def;
}

UnicodeString FloatToStr(double v)
{
  // Delphi FloatToStr uses 15 significant digits ("General" format)
  wchar_t buf[64];
  swprintf(buf, 64, L"%.15G", v);
  return UnicodeString(buf);
}

UnicodeString TrimLeft(const UnicodeString& s)
{
  const int l = s.Length();
  int i = 1;
  while (i <= l && s[i] <= ' ') i++;
  return s.SubString(i, l - i + 1);
}

UnicodeString TrimRight(const UnicodeString& s)
{
  int l = s.Length();
  while (l > 0 && s[l] <= ' ') l--;
  return s.SubString(1, l);
}

UnicodeString Trim(const UnicodeString& s)
{
  int l = s.Length(), i = 1;
  if (l > 0 && s[i] > ' ' && s[l] > ' ')
    return s;
  while (i <= l && s[i] <= ' ') i++;
  if (i > l)
    return UnicodeString();
  while (s[l] <= ' ') l--;
  return s.SubString(i, l - i + 1);
}

UnicodeString LowerCase(const UnicodeString& s)
{
  UnicodeString r(s);
  for (auto& ch : r)
    if (ch >= 'A' && ch <= 'Z')
      ch += 'a' - 'A';
  return r;
}

UnicodeString UpperCase(const UnicodeString& s)
{
  UnicodeString r(s);
  for (auto& ch : r)
    if (ch >= 'a' && ch <= 'z')
      ch -= 'a' - 'A';
  return r;
}

UnicodeString AnsiLowerCase(const UnicodeString& s)
{
  UnicodeString r(s);
  if (!r.IsEmpty())
    CharLowerBuffW(r.FirstChar(), r.Length());
  return r;
}

UnicodeString AnsiUpperCase(const UnicodeString& s)
{
  UnicodeString r(s);
  if (!r.IsEmpty())
    CharUpperBuffW(r.FirstChar(), r.Length());
  return r;
}

int CompareText(const UnicodeString& a, const UnicodeString& b)
{
  // ASCII case-insensitive ordinal comparison (System.SysUtils.CompareText)
  UnicodeString la = LowerCase(a), lb = LowerCase(b);
  return la.Compare(lb);
}

bool SameText(const UnicodeString& a, const UnicodeString& b)
{
  return CompareText(a, b) == 0;
}

int LastDelimiter(const UnicodeString& delims, const UnicodeString& s)
{
  for (int i = s.Length(); i >= 1; i--) {
    if (wcschr(delims.c_str(), s[i]) != nullptr)
      return i;
  }
  return 0;
}

// PathDelim = '\', DriveDelim = ':' (Windows RTL)
UnicodeString ExtractFilePath(const UnicodeString& s)
{
  int i = LastDelimiter(L"\\:", s);
  return s.SubString(1, i);
}

UnicodeString ExtractFileDir(const UnicodeString& s)
{
  int i = LastDelimiter(L"\\:", s);
  if (i > 1 && s[i] == '\\' && LastDelimiter(L"\\:", s.SubString(1, i - 1)) != i - 1)
    i--;
  return s.SubString(1, i);
}

UnicodeString ExtractFileName(const UnicodeString& s)
{
  int i = LastDelimiter(L"\\:", s);
  return s.SubString(i + 1, INT_MAX);
}

UnicodeString ExtractFileExt(const UnicodeString& s)
{
  int i = LastDelimiter(L".\\:", s);
  if (i > 0 && s[i] == '.')
    return s.SubString(i, INT_MAX);
  return UnicodeString();
}

UnicodeString ChangeFileExt(const UnicodeString& s, const UnicodeString& ext)
{
  int i = LastDelimiter(L".\\:", s);
  if (i == 0 || s[i] != '.')
    i = INT_MAX;
  return s.SubString(1, i - 1) + ext;
}

UnicodeString ExpandFileName(const UnicodeString& s)
{
  wchar_t buf[MAX_PATH * 2];
  DWORD n = GetFullPathNameW(s.c_str(), MAX_PATH * 2, buf, nullptr);
  if (n == 0 || n >= MAX_PATH * 2)
    return s;
  return UnicodeString(buf, static_cast<int>(n));
}

UnicodeString IncludeTrailingPathDelimiter(const UnicodeString& s)
{
  if (!s.IsEmpty() && s[s.Length()] == '\\')
    return s;
  return s + L"\\";
}

bool FileExists(const UnicodeString& s)
{
  DWORD attr = GetFileAttributesW(s.c_str());
  return attr != INVALID_FILE_ATTRIBUTES && !(attr & FILE_ATTRIBUTE_DIRECTORY);
}

__int64 DiskFree(Byte drive)
{
  ULARGE_INTEGER avail, total, freeBytes;
  wchar_t root[4] = { L'A', L':', L'\\', 0 };
  const wchar_t* pRoot = nullptr;
  if (drive != 0) {
    root[0] = L'A' + drive - 1;
    pRoot = root;
  }
  if (!GetDiskFreeSpaceExW(pRoot, &avail, &total, &freeBytes))
    return -1;
  return static_cast<__int64>(avail.QuadPart);
}

UnicodeString SysErrorMessage(DWORD err)
{
  wchar_t* pBuf = nullptr;
  DWORD n = FormatMessageW(FORMAT_MESSAGE_ALLOCATE_BUFFER |
    FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS, nullptr, err,
    0, reinterpret_cast<wchar_t*>(&pBuf), 0, nullptr);
  UnicodeString s;
  if (n != 0 && pBuf != nullptr) {
    s = UnicodeString(pBuf, static_cast<int>(n));
    LocalFree(pBuf);
  }
  return TrimRight(s);
}

static UnicodeString replaceImpl(const UnicodeString& s, const UnicodeString& from,
  const UnicodeString& to, bool blIgnoreCase)
{
  if (from.IsEmpty() || s.IsEmpty())
    return s;
  std::wstring src = s.std_str(), pat = from.std_str(), rep = to.std_str();
  std::wstring srcCmp = src, patCmp = pat;
  if (blIgnoreCase) {
    srcCmp = AnsiUpperCase(s).std_str();
    patCmp = AnsiUpperCase(from).std_str();
  }
  std::wstring out;
  size_t pos = 0;
  while (true) {
    size_t f = srcCmp.find(patCmp, pos);
    if (f == std::wstring::npos)
      break;
    out.append(src, pos, f - pos);
    out.append(rep);
    pos = f + pat.length();
  }
  out.append(src, pos, std::wstring::npos);
  return UnicodeString(out);
}

UnicodeString ReplaceStr(const UnicodeString& s, const UnicodeString& from,
  const UnicodeString& to)
{
  return replaceImpl(s, from, to, false);
}

UnicodeString ReplaceText(const UnicodeString& s, const UnicodeString& from,
  const UnicodeString& to)
{
  return replaceImpl(s, from, to, true);
}

void ShowMessage(const UnicodeString& s)
{
  fwprintf(stderr, L"ShowMessage: %ls\n", s.c_str());
}

// ---------------------------------------------------------------------------
UnicodeString Format(const UnicodeString& fmt, const std::vector<TVarRec>& args)
{
  std::wstring out;
  size_t argIdx = 0;
  const std::wstring& f = fmt.std_str();
  for (size_t i = 0; i < f.length(); i++) {
    if (f[i] != '%') {
      out.push_back(f[i]);
      continue;
    }
    if (i + 1 < f.length() && f[i + 1] == '%') {
      out.push_back('%');
      i++;
      continue;
    }
    // %[-][width][.prec]type
    std::wstring spec = L"%";
    size_t j = i + 1;
    while (j < f.length() && (f[j] == '-' || (f[j] >= '0' && f[j] <= '9') || f[j] == '.'))
      spec.push_back(f[j++]);
    if (j >= f.length())
      break;
    wchar_t type = towlower(f[j]);
    i = j;
    if (argIdx >= args.size())
      throw EConvertError("Format '" + fmt + "' invalid or incompatible with argument");
    const TVarRec& a = args[argIdx++];
    wchar_t buf[128];
    switch (type) {
    case 'd':
    case 'u':
      spec += L"lld";
      swprintf(buf, 128, spec.c_str(), static_cast<long long>(a.i));
      out += buf;
      break;
    case 'x':
      spec += L"llX";
      swprintf(buf, 128, spec.c_str(), static_cast<long long>(a.i));
      out += buf;
      break;
    case 'f':
    case 'g':
    case 'e':
    case 'n':
      spec += (type == 'n') ? L'f' : type;
      swprintf(buf, 128, spec.c_str(), a.kind == TVarRec::vtDouble ? a.d :
        static_cast<double>(a.i));
      out += buf;
      break;
    case 's':
    default:
      out += a.s.std_str();
    }
  }
  return UnicodeString(out);
}

// ---------------------------------------------------------------------------
// System.Ln (Delphi Win64 RTL) as linked into PwTech.exe 3.6.1 (function at
// VA 0x53e6b0, kernel at 0x53e5c0, constants at 0xe3ed50..0xe3ede0), i.e.
// the implementation behind Math.Log2 (VA 0x589250: Ln(x) * 0x3FF71547652B82FE).
// Reconstructed operation by operation from the disassembly; compiled without
// FP contraction so that every step rounds like the original SSE2 code.
// Only the finite positive path is reproduced exactly; special values fall
// back to the IEEE results (the RTL additionally raises FP exceptions there).

static double bitsToDouble(unsigned long long u)
{
  double d;
  memcpy(&d, &u, 8);
  return d;
}

// log(1+f) for f in [sqrt(2)/2-1, sqrt(2)-1]
static double delphiLnKernel(double f)
{
  static const double P[12] = {
    bitsToDouble(0x3fb0c835bd538f8cULL), bitsToDouble(0x3fa988ab1ef94f29ULL),
    bitsToDouble(0x3fae3c098f53b5eeULL), bitsToDouble(0x3fb11053fca919d4ULL),
    bitsToDouble(0x3fb3b140d1e4050fULL), bitsToDouble(0x3fb745d159460dd7ULL),
    bitsToDouble(0x3fbc71c71cbb0290ULL), bitsToDouble(0x3fc249249248f262ULL),
    bitsToDouble(0x3c602e384367005fULL), bitsToDouble(0x3fc99999999999b4ULL),
    bitsToDouble(0x3c750aaac32dac90ULL), bitsToDouble(0x3fd5555555555555ULL)
  };
  volatile double vf = f;   // keep the evaluation order exactly as below
  const double ff = vf;
  const double s = ff / (ff + 2.0);
  const double halfF = ff * 0.5;
  const double hfsq = ff * halfF;
  const double x3 = hfsq * halfF;
  const double z = s * s;
  const double w = z * z;
  double t1 = P[0] * w + P[2];
  double t2 = P[1] * w + P[3];
  t1 = t1 * w + P[4];
  t2 = t2 * w + P[5];
  t1 = t1 * w + P[6];
  t2 = t2 * w + P[7];
  t1 = t1 * w + P[8];
  t1 = t1 + P[9];
  t2 = t2 * w + P[10];
  t1 = t1 * z;
  double r = t2 + t1;
  r = r + P[11];
  r = r * 2.0;
  r = r * z;
  r = r * s;
  const double sx3 = s * x3;
  r = r + x3;
  r = r - hfsq;
  r = r - sx3;
  r = r + ff;
  return r;
}

double DelphiLn(double x)
{
  unsigned long long bits;
  memcpy(&bits, &x, 8);
  if ((bits >> 63) != 0 || bits == 0 || bits >= 0x7ff0000000000000ULL)
    return std::log(x);   // non-finite / non-positive: IEEE result

  int k;
  unsigned long long mant = bits & 0xfffffffffffffULL;
  unsigned exponent = static_cast<unsigned>(bits >> 52) & 0x7ff;
  if (exponent == 0) {
    // subnormal: normalise so that bit 52 is set
    k = -1022;
    while (!(mant & 0x10000000000000ULL)) {
      mant <<= 1;
      k--;
    }
    mant &= 0xfffffffffffffULL;
  }
  else
    k = static_cast<int>(exponent) - 1023;

  double m;
  if (mant <= 0x6a09e667f3bccULL)
    m = bitsToDouble(0x3ff0000000000000ULL + mant);   // [1, sqrt 2]
  else {
    m = bitsToDouble(0x3fe0000000000000ULL + mant);   // [sqrt(2)/2, 1)
    k++;
  }

  double r = delphiLnKernel(m - 1.0);
  const double dk = static_cast<double>(k);
  r = r + dk * bitsToDouble(0x3e37d1cf79abc9e4ULL);   // ln2_lo
  r = r + dk * bitsToDouble(0x3fe62e42fc000000ULL);   // ln2_hi
  return r;
}
