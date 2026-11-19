// Minimal replacement for the parts of the C++Builder RTL/VCL that the
// Password Tech core units use. All VCL "headers" in this directory just
// include this file.
//
// Fidelity notes (see README.md, "Shim deviations"):
//  * UnicodeString/AnsiString: 1-based operator[] with range check, Trim()
//    removes chars <= ' ' (Delphi semantics), SubString/Pos/Delete/Insert follow
//    Delphi Copy/Pos/Delete/Insert. No copy-on-write (irrelevant here).
//  * LowerCase/UpperCase (free functions) convert ASCII A-Z only, like
//    System.SysUtils.LowerCase/UpperCase. AnsiLowerCase/AnsiUpperCase use
//    CharLowerBuffW/CharUpperBuffW, like the RTL on Windows.
//  * char* <-> UnicodeString conversions go through CP_ACP, like the RTL.
//  * Log2 = Ln(x) * 1/ln(2) with the RTL's own Ln (reconstructed from the
//    original binary), not std::log2 - see vclshim.cpp.
#pragma once

#include <windows.h>
#include <string>
#include <cstring>
#include <cwchar>
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <stdexcept>
#include <vector>
#include <memory>
#include <algorithm>
#include <utility>
// the C++Builder RTL headers transitively provide these
#include <map>
#include <set>
#include <unordered_map>
#include <unordered_set>
#include <atomic>
#include <functional>
#include <array>
#include <tuple>
#include <limits>
#include <climits>
#include <cfloat>
#include <intrin.h>

// ---------------------------------------------------------------------------
// basic Delphi types
typedef unsigned short Word;
typedef unsigned char Byte;
typedef unsigned int Cardinal;
typedef int Integer;
typedef wchar_t WideChar;
typedef char AnsiChar;
typedef bool Boolean;
typedef double Extended;   // Win64 RTL: Extended == Double
typedef __int64 Int64;
typedef unsigned __int64 UInt64;

class AnsiString;

// ---------------------------------------------------------------------------
class UnicodeString
{
  std::wstring m_s;

  void CheckIndex(int idx) const;

public:
  UnicodeString() {}
  UnicodeString(const UnicodeString&) = default;
  UnicodeString(UnicodeString&&) = default;
  UnicodeString& operator=(const UnicodeString&) = default;
  UnicodeString& operator=(UnicodeString&&) = default;

  UnicodeString(const wchar_t* p) : m_s(p ? p : L"") {}
  UnicodeString(const wchar_t* p, int len) : m_s(p, len > 0 ? len : 0) {}
  UnicodeString(const char* p);            // CP_ACP
  UnicodeString(const char* p, int len);   // CP_ACP
  UnicodeString(const AnsiString& s);      // CP_ACP
  UnicodeString(const std::wstring& s) : m_s(s) {}
  UnicodeString(wchar_t c) : m_s(1, c) {}
  explicit UnicodeString(char c);
  explicit UnicodeString(int v) : m_s(std::to_wstring(v)) {}
  explicit UnicodeString(unsigned v) : m_s(std::to_wstring(v)) {}
  explicit UnicodeString(long v) : m_s(std::to_wstring(v)) {}
  explicit UnicodeString(unsigned long v) : m_s(std::to_wstring(v)) {}
  explicit UnicodeString(__int64 v) : m_s(std::to_wstring(v)) {}
  explicit UnicodeString(unsigned __int64 v) : m_s(std::to_wstring(v)) {}

  int Length() const { return static_cast<int>(m_s.length()); }
  bool IsEmpty() const { return m_s.empty(); }
  wchar_t* c_str() const { return const_cast<wchar_t*>(m_s.c_str()); }
  wchar_t* w_str() const { return c_str(); }
  wchar_t* data() { return IsEmpty() ? nullptr : &m_s[0]; }
  wchar_t* FirstChar() { return IsEmpty() ? nullptr : &m_s[0]; }
  const wchar_t* FirstChar() const { return IsEmpty() ? nullptr : m_s.c_str(); }
  wchar_t* LastChar() { return IsEmpty() ? nullptr : &m_s[m_s.length() - 1]; }
  const std::wstring& std_str() const { return m_s; }

  UnicodeString& SetLength(int n);
  UnicodeString& Delete(int index, int count);
  UnicodeString& Insert(const UnicodeString& s, int index);
  int Pos(const UnicodeString& sub) const;
  UnicodeString SubString(int index, int count) const;
  UnicodeString Trim() const;
  UnicodeString TrimLeft() const;
  UnicodeString TrimRight() const;
  UnicodeString LowerCase() const;   // = AnsiLowerCase (RTL behaviour)
  UnicodeString UpperCase() const;   // = AnsiUpperCase (RTL behaviour)
  int ToInt() const;
  int ToIntDef(int def) const;
  int Compare(const UnicodeString& o) const { return m_s.compare(o.m_s); }
  int CompareIC(const UnicodeString& o) const;

  wchar_t& operator[](int idx) { CheckIndex(idx); return m_s[idx - 1]; }
  wchar_t operator[](int idx) const { CheckIndex(idx); return m_s[idx - 1]; }

  wchar_t* begin() { return IsEmpty() ? nullptr : &m_s[0]; }
  wchar_t* end() { return IsEmpty() ? nullptr : &m_s[0] + m_s.length(); }
  const wchar_t* begin() const { return IsEmpty() ? nullptr : m_s.c_str(); }
  const wchar_t* end() const { return IsEmpty() ? nullptr : m_s.c_str() + m_s.length(); }

  UnicodeString& operator+=(const UnicodeString& o) { m_s += o.m_s; return *this; }
  UnicodeString& operator+=(wchar_t c) { m_s += c; return *this; }
  UnicodeString& operator+=(const wchar_t* p) { m_s += p; return *this; }

  bool operator==(const UnicodeString& o) const { return m_s == o.m_s; }
  bool operator!=(const UnicodeString& o) const { return m_s != o.m_s; }
  bool operator<(const UnicodeString& o) const { return m_s < o.m_s; }
  bool operator>(const UnicodeString& o) const { return m_s > o.m_s; }
  bool operator<=(const UnicodeString& o) const { return m_s <= o.m_s; }
  bool operator>=(const UnicodeString& o) const { return m_s >= o.m_s; }
};

inline UnicodeString operator+(const UnicodeString& a, const UnicodeString& b)
{
  UnicodeString r(a);
  r += b;
  return r;
}
inline UnicodeString operator+(const UnicodeString& a, const wchar_t* b) { return a + UnicodeString(b); }
inline UnicodeString operator+(const wchar_t* a, const UnicodeString& b) { return UnicodeString(a) + b; }
inline UnicodeString operator+(const UnicodeString& a, const char* b) { return a + UnicodeString(b); }
inline UnicodeString operator+(const char* a, const UnicodeString& b) { return UnicodeString(a) + b; }
inline UnicodeString operator+(const UnicodeString& a, wchar_t b) { return a + UnicodeString(b); }
inline bool operator==(const UnicodeString& a, const wchar_t* b) { return a == UnicodeString(b); }
inline bool operator==(const UnicodeString& a, const char* b) { return a == UnicodeString(b); }
inline bool operator!=(const UnicodeString& a, const wchar_t* b) { return a != UnicodeString(b); }
inline bool operator!=(const UnicodeString& a, const char* b) { return a != UnicodeString(b); }

typedef UnicodeString String;

// ---------------------------------------------------------------------------
class AnsiString
{
  std::string m_s;

  void CheckIndex(int idx) const;

public:
  AnsiString() {}
  AnsiString(const AnsiString&) = default;
  AnsiString(AnsiString&&) = default;
  AnsiString& operator=(const AnsiString&) = default;
  AnsiString& operator=(AnsiString&&) = default;

  AnsiString(const char* p) : m_s(p ? p : "") {}
  AnsiString(const char* p, int len) : m_s(p, len > 0 ? len : 0) {}
  AnsiString(const std::string& s) : m_s(s) {}
  AnsiString(const UnicodeString& s);   // CP_ACP
  AnsiString(const wchar_t* p);         // CP_ACP
  AnsiString(char c) : m_s(1, c) {}
  explicit AnsiString(int v) : m_s(std::to_string(v)) {}

  int Length() const { return static_cast<int>(m_s.length()); }
  bool IsEmpty() const { return m_s.empty(); }
  char* c_str() const { return const_cast<char*>(m_s.c_str()); }
  char* data() { return IsEmpty() ? nullptr : &m_s[0]; }
  const std::string& std_str() const { return m_s; }
  AnsiString& SetLength(int n) { m_s.resize(n > 0 ? n : 0, '\0'); return *this; }
  AnsiString SubString(int index, int count) const;
  AnsiString Trim() const;

  char& operator[](int idx) { CheckIndex(idx); return m_s[idx - 1]; }
  char operator[](int idx) const { CheckIndex(idx); return m_s[idx - 1]; }

  AnsiString& operator+=(const AnsiString& o) { m_s += o.m_s; return *this; }
  bool operator==(const AnsiString& o) const { return m_s == o.m_s; }
  bool operator!=(const AnsiString& o) const { return m_s != o.m_s; }
  bool operator<(const AnsiString& o) const { return m_s < o.m_s; }
};

inline AnsiString operator+(const AnsiString& a, const AnsiString& b)
{
  AnsiString r(a);
  r += b;
  return r;
}

typedef AnsiString RawByteString;
typedef AnsiString UTF8String;   // only used as a byte container here

// ---------------------------------------------------------------------------
// Exceptions (System.SysUtils / System.Classes hierarchy)
class Exception
{
public:
  UnicodeString Message;
  Exception(const UnicodeString& sMsg) : Message(sMsg) {}
  virtual ~Exception() {}
  virtual const char* ClassName() const { return "Exception"; }
};

#define PT_DECLARE_EXC(Name, Base) \
  class Name : public Base { \
  public: \
    Name(const UnicodeString& sMsg) : Base(sMsg) {} \
    const char* ClassName() const override { return #Name; } \
  };

PT_DECLARE_EXC(EAbort, Exception)
PT_DECLARE_EXC(EConvertError, Exception)
PT_DECLARE_EXC(EHeapException, Exception)
PT_DECLARE_EXC(EOutOfMemory, EHeapException)
PT_DECLARE_EXC(EExternal, Exception)
PT_DECLARE_EXC(EIntError, EExternal)
PT_DECLARE_EXC(ERangeError, EIntError)
PT_DECLARE_EXC(EOSError, Exception)
PT_DECLARE_EXC(EInvalidOperation, Exception)
PT_DECLARE_EXC(EStreamError, Exception)
PT_DECLARE_EXC(EFileStreamError, EStreamError)
PT_DECLARE_EXC(EFCreateError, EFileStreamError)
PT_DECLARE_EXC(EFOpenError, EFileStreamError)
PT_DECLARE_EXC(EFilerError, EStreamError)
PT_DECLARE_EXC(EReadError, EFilerError)
PT_DECLARE_EXC(EWriteError, EFilerError)
PT_DECLARE_EXC(EClipboardException, Exception)
PT_DECLARE_EXC(EStringListError, Exception)

[[noreturn]] void OutOfMemoryError();

// ---------------------------------------------------------------------------
// Streams (System.Classes)
const Word fmCreate         = 0xFF00;
const Word fmOpenRead       = 0x0000;
const Word fmOpenWrite      = 0x0001;
const Word fmOpenReadWrite  = 0x0002;
const Word fmShareCompat    = 0x0000;
const Word fmShareExclusive = 0x0010;
const Word fmShareDenyWrite = 0x0020;
const Word fmShareDenyRead  = 0x0030;
const Word fmShareDenyNone  = 0x0040;

const Word soFromBeginning = 0;
const Word soFromCurrent   = 1;
const Word soFromEnd       = 2;
enum TSeekOrigin { soBeginning, soCurrent, soEnd };

class TObject
{
public:
  virtual ~TObject() {}
};

class TStream : public TObject
{
public:
  virtual int Read(void* pBuf, int nCount) = 0;
  virtual int Write(const void* pBuf, int nCount) = 0;
  virtual __int64 Seek(__int64 offset, int origin) = 0;
  void ReadBuffer(void* pBuf, int nCount);
  void WriteBuffer(const void* pBuf, int nCount);

  __int64 GetPosition() { return Seek(0, soFromCurrent); }
  void SetPosition(__int64 v) { Seek(v, soFromBeginning); }
  virtual __int64 GetSize();
  virtual void SetSize(__int64 v) {}
  __declspec(property(get=GetPosition, put=SetPosition)) __int64 Position;
  __declspec(property(get=GetSize, put=SetSize)) __int64 Size;
};

class THandleStream : public TStream
{
protected:
  HANDLE m_hFile;
public:
  explicit THandleStream(HANDLE h) : m_hFile(h) {}
  int Read(void* pBuf, int nCount) override;
  int Write(const void* pBuf, int nCount) override;
  __int64 Seek(__int64 offset, int origin) override;
  void SetSize(__int64 v) override;
  HANDLE GetHandle() const { return m_hFile; }
};

class TFileStream : public THandleStream
{
  UnicodeString m_sFileName;
public:
  TFileStream(const UnicodeString& sFileName, Word wMode);
  ~TFileStream() override;
  UnicodeString GetFileName() const { return m_sFileName; }
  __declspec(property(get=GetFileName)) UnicodeString FileName;
};

class TMemoryStream : public TStream
{
  std::vector<char> m_buf;
  __int64 m_pos = 0;
public:
  int Read(void* pBuf, int nCount) override;
  int Write(const void* pBuf, int nCount) override;
  __int64 Seek(__int64 offset, int origin) override;
  void SetSize(__int64 v) override { m_buf.resize(static_cast<size_t>(v)); if (m_pos > v) m_pos = v; }
  void* Memory() { return m_buf.empty() ? nullptr : m_buf.data(); }
};

// ---------------------------------------------------------------------------
// System.SysUtils functions
UnicodeString IntToStr(int v);
UnicodeString IntToStr(__int64 v);
UnicodeString UIntToStr(unsigned v);
int StrToInt(const UnicodeString& s);
int StrToIntDef(const UnicodeString& s, int def);
UnicodeString FloatToStr(double v);
UnicodeString Trim(const UnicodeString& s);
UnicodeString TrimLeft(const UnicodeString& s);
UnicodeString TrimRight(const UnicodeString& s);
UnicodeString LowerCase(const UnicodeString& s);   // ASCII only (RTL default)
UnicodeString UpperCase(const UnicodeString& s);   // ASCII only (RTL default)
UnicodeString AnsiLowerCase(const UnicodeString& s);
UnicodeString AnsiUpperCase(const UnicodeString& s);
int CompareText(const UnicodeString& a, const UnicodeString& b);
bool SameText(const UnicodeString& a, const UnicodeString& b);
UnicodeString ExtractFilePath(const UnicodeString& s);
UnicodeString ExtractFileDir(const UnicodeString& s);
UnicodeString ExtractFileName(const UnicodeString& s);
UnicodeString ExtractFileExt(const UnicodeString& s);
UnicodeString ChangeFileExt(const UnicodeString& s, const UnicodeString& ext);
UnicodeString ExpandFileName(const UnicodeString& s);
UnicodeString IncludeTrailingPathDelimiter(const UnicodeString& s);
bool FileExists(const UnicodeString& s);
int LastDelimiter(const UnicodeString& delims, const UnicodeString& s);
__int64 DiskFree(Byte drive);
UnicodeString SysErrorMessage(DWORD err);

// StrUtils
UnicodeString ReplaceStr(const UnicodeString& s, const UnicodeString& from,
  const UnicodeString& to);
UnicodeString ReplaceText(const UnicodeString& s, const UnicodeString& from,
  const UnicodeString& to);

// Math
// System.Ln / System.Math.Log2 exactly as in the original binary (Win64 RTL,
// pure-Pascal Ln + multiplication by 1/ln(2)); reconstructed from the
// disassembly of PwTech.exe 3.6.1, see vclshim.cpp. NOT std::log2.
double DelphiLn(double x);
inline double Log2(double x) { return DelphiLn(x) * 1.4426950408889634; }

// Dialogs
void ShowMessage(const UnicodeString& s);

// ---------------------------------------------------------------------------
// Format() with ARRAYOFCONST - a small TVarRec emulation supporting the
// argument kinds used by the compiled units (%d, %u, %s, %x, %%).
struct TVarRec
{
  enum Kind { vtInt, vtInt64, vtDouble, vtUStr, vtPChar, vtPWChar } kind;
  __int64 i = 0;
  double d = 0;
  UnicodeString s;
  TVarRec(int v) : kind(vtInt), i(v) {}
  TVarRec(unsigned v) : kind(vtInt64), i(v) {}
  TVarRec(long v) : kind(vtInt), i(v) {}
  TVarRec(__int64 v) : kind(vtInt64), i(v) {}
  TVarRec(double v) : kind(vtDouble), d(v) {}
  TVarRec(const UnicodeString& v) : kind(vtUStr), s(v) {}
  TVarRec(const char* v) : kind(vtPChar), s(v) {}
  TVarRec(const wchar_t* v) : kind(vtPWChar), s(v) {}
};
UnicodeString Format(const UnicodeString& fmt, const std::vector<TVarRec>& args);
#define ARRAYOFCONST(values) std::vector<TVarRec>{ PT_UNPAREN values }
#define PT_UNPAREN(...) __VA_ARGS__

// ---------------------------------------------------------------------------
// Winapi.Messages
struct TMessage
{
  Cardinal Msg;
  WPARAM WParam;
  LPARAM LParam;
  LRESULT Result;
};

// SEH: see __except in prelude.h; the filter code is never executed
#undef GetExceptionCode
#define GetExceptionCode() 0UL

// ---------------------------------------------------------------------------
// System.Classes.TThread / System.SyncObjs (only what Scripting.cpp needs;
// the harness itself runs scripts synchronously and never starts a thread)
enum TWaitResult { wrSignaled, wrTimeout, wrAbandoned, wrError, wrIOCompletion };
enum TThreadPriority { tpIdle, tpLowest, tpLower, tpNormal, tpHigher, tpHighest,
  tpTimeCritical };

class THandleObject : public TObject
{
protected:
  HANDLE m_hHandle = nullptr;
  int m_nLastError = 0;
public:
  ~THandleObject() override;
  int GetLastErrorValue() const { return m_nLastError; }
  HANDLE GetHandleValue() const { return m_hHandle; }
  __declspec(property(get=GetLastErrorValue)) int LastError;
  __declspec(property(get=GetHandleValue)) HANDLE Handle;
};

class TEvent : public THandleObject
{
public:
  TEvent(void* pAttr, bool blManualReset, bool blInitialState,
    const UnicodeString& sName, bool blUseCOMWait = false);
  TWaitResult WaitFor(Cardinal timeout = INFINITE);
  void SetEvent();
  void ResetEvent();
};

class TSimpleEvent : public TEvent
{
public:
  TSimpleEvent(void* pAttr, bool blManualReset, bool blInitialState,
    const UnicodeString& sName, bool blUseCOMWait = false)
    : TEvent(pAttr, blManualReset, blInitialState, sName, blUseCOMWait) {}
  explicit TSimpleEvent(bool blUseCOMWait = false)
    : TEvent(nullptr, true, false, UnicodeString(), blUseCOMWait) {}
};

class TThread : public TObject
{
  HANDLE m_hThread = nullptr;
  static DWORD WINAPI ThreadProc(LPVOID p);
protected:
  volatile bool Terminated = false;
  virtual void Execute() = 0;
public:
  bool FreeOnTerminate = false;
  TThreadPriority Priority = tpNormal;
  volatile bool m_blFinished = false;
  explicit TThread(bool blCreateSuspended);
  ~TThread() override;
  void Start();
  void Terminate() { Terminated = true; }
  Cardinal WaitFor();
  bool GetFinished() const { return m_blFinished; }
  HANDLE GetHandleValue() const { return m_hThread; }
  __declspec(property(get=GetFinished)) bool Finished;
  __declspec(property(get=GetHandleValue)) HANDLE Handle;
};
