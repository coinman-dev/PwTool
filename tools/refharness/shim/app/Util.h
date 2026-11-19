// Harness replacement for upstream src/util/Util.h (GUI-bound: TForm, TCGauge,
// edit boxes, ...). Declares the non-GUI helpers used by the compiled core
// units. Implementations are copied verbatim from upstream Util.cpp where the
// logic matters (SplitStringBuf, FloorEntropyBits, CppStdExceptionToString);
// the clipboard functions write into an in-process buffer instead of the real
// Windows clipboard (see app_shim.cpp).
#ifndef UtilH
#define UtilH
#include <vcl.h>
#include <tuple>
#include <vector>
#include "SecureMem.h"
#include "UnicodeUtil.h"

void OutOfDiskSpaceError(void);

SecureWString GetClipboardTextBuf(void);
SecureAnsiString GetClipboardTextBufAnsi(void);
void SetClipboardTextBuf(const wchar_t* pwszSrc);
void SetClipboardTextBufAnsi(const char* pszSrc);

WString CppStdExceptionToString(const std::exception& e);
int FloorEntropyBits(double val);
std::vector<SecureWString> SplitStringBuf(const wchar_t* pwzsSrc,
  const wchar_t* pwszSep);

// --- harness-only: access to the captured "clipboard" ---
struct CapturedClipboard {
  bool blSet = false;
  bool blViaAnsi = false;          // set through SetClipboardTextBufAnsi()
  std::string asAnsiBytes;         // raw bytes passed to SetClipboardTextBufAnsi()
  std::wstring sText;              // UTF-16 text as it would be on the clipboard
};
extern CapturedClipboard g_capturedClipboard;

#endif
