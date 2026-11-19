// Implementations for the harness replacements of Main.h, Language.h and
// Util.h (see the headers in this directory).
#include <vcl.h>
#include <exception>
#include <stdexcept>
#include <new>
#include "Main.h"
#include "Language.h"
#include "Util.h"

WString g_sExePath;
Configuration g_config;
WString g_sNewline = "\r\n";

CapturedClipboard g_capturedClipboard;

// ---------------------------------------------------------------------------
// Language.h: no translation
WString TRL(const AnsiString& asStr) { return WString(asStr); }
WString TRL(const WString& sStr) { return sStr; }
WString TRL(const char* pszStr, const char*) { return WString(pszStr); }
WString TRL(const wchar_t* pwszStr) { return WString(pwszStr); }
WString TRLDEF(const AnsiString&, const AnsiString& asDefault) { return WString(asDefault); }
void TRLS(WString&) {}
WString TRLFormat(const WString& sFormat, const std::vector<WString>& args)
{
  return FormatW(sFormat, args);
}

// ---------------------------------------------------------------------------
// Util.cpp:112
void OutOfDiskSpaceError(void)
{
  throw EStreamError(TRL("Out of disk space"));
}

// Clipboard (Util.cpp:205-290) -> in-process capture buffer.
SecureWString GetClipboardTextBuf(void)
{
  SecureWString sText;
  if (g_capturedClipboard.blSet && !g_capturedClipboard.sText.empty())
    sText.AssignStr(g_capturedClipboard.sText.c_str());
  return sText;
}

SecureAnsiString GetClipboardTextBufAnsi(void)
{
  // Windows synthesises CF_TEXT from CF_UNICODETEXT using CP_ACP
  SecureAnsiString asText;
  if (g_capturedClipboard.blSet && !g_capturedClipboard.sText.empty()) {
    const wchar_t* p = g_capturedClipboard.sText.c_str();
    int n = WideCharToMultiByte(CP_ACP, 0, p, -1, nullptr, 0, nullptr, nullptr);
    if (n > 0) {
      asText.New(n);
      WideCharToMultiByte(CP_ACP, 0, p, -1, asText, n, nullptr, nullptr);
    }
  }
  return asText;
}

void SetClipboardTextBuf(const wchar_t* pwszSrc)
{
  g_capturedClipboard.blSet = true;
  g_capturedClipboard.blViaAnsi = false;
  g_capturedClipboard.asAnsiBytes.clear();
  g_capturedClipboard.sText = pwszSrc ? pwszSrc : L"";
}

void SetClipboardTextBufAnsi(const char* pszSrc)
{
  g_capturedClipboard.blSet = true;
  g_capturedClipboard.blViaAnsi = true;
  g_capturedClipboard.asAnsiBytes = pszSrc ? pszSrc : "";
  g_capturedClipboard.sText.clear();
  // same conversion as Util.cpp:282-288
  word32 lLen = MultiByteToWideChar(CP_ACP, 0, pszSrc, -1, nullptr, 0);
  if (lLen > 0) {
    SecureWString wideSrc(lLen);
    MultiByteToWideChar(CP_ACP, 0, pszSrc, -1, wideSrc, lLen);
    g_capturedClipboard.sText = wideSrc.c_str();
  }
}

// ---------------------------------------------------------------------------
// verbatim from Util.cpp:535
WString CppStdExceptionToString(const std::exception& e)
{
  WString sMsg;

  if (dynamic_cast<const std::bad_alloc*>(&e))
    sMsg = "std::bad_alloc: ";
  else if (dynamic_cast<const std::bad_exception*>(&e))
    sMsg = "std::bad_exception: ";
  else if (dynamic_cast<const std::logic_error*>(&e))
    sMsg = "std::logic_error: ";
  else if (dynamic_cast<const std::runtime_error*>(&e))
    sMsg = "std::runtime_error: ";
  else
    sMsg = "std::exception: ";

  sMsg += e.what();

  return sMsg;
}

// verbatim from Util.cpp:617
std::vector<SecureWString> SplitStringBuf(const wchar_t* pwszSrc,
  const wchar_t* pwszSep)
{
  std::vector<SecureWString> result;
  while (*pwszSrc != '\0') {
    word32 lLen = wcscspn(pwszSrc, pwszSep);
    if (lLen > 0) {
      SecureWString sItem(pwszSrc, lLen + 1);
      sItem[lLen] = '\0';
      result.push_back(std::move(sItem));
    }
    pwszSrc += lLen;
    if (*pwszSrc != '\0')
      pwszSrc++; // move pointer beyond separator character
  }
  return result;
}

// verbatim from Util.cpp:654
int FloorEntropyBits(double val)
{
  double intpart;
  double frac = modf(val, &intpart);
  if (frac > 0.99)
    intpart += 1;
  return static_cast<int>(intpart);
}
