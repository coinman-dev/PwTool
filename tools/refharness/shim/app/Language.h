// Harness replacement for upstream src/main/Language.h (GUI-bound).
// Translation is a no-op: TRL() returns its argument (English UI strings).
#ifndef LanguageH
#define LanguageH
#include <vector>
#include "UnicodeUtil.h"

WString TRL(const AnsiString& asStr);
WString TRL(const WString& sStr);
WString TRL(const char* pszStr, const char* pszContext = nullptr);
WString TRL(const wchar_t* pwszStr);
WString TRLDEF(const AnsiString& asStr, const AnsiString& asDefault);
void TRLS(WString& sStr);
WString TRLFormat(const WString& sFormat,
  const std::vector<WString>& args);

#endif
