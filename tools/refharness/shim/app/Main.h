// Harness replacement for upstream src/main/Main.h (GUI-bound).
// Declares only the globals used by the compiled core units.
#ifndef MainH
#define MainH
#include <unordered_set>
#include <atomic>
#include <optional>
#include "UnicodeUtil.h"
#include "RandomPool.h"
#include "PasswGen.h"
#include "RandomGenerator.h"
#include "Configuration.h"
#include "EntropyManager.h"

extern WString g_sExePath;     // directory of the executable, with trailing '\'
extern Configuration g_config;
extern WString g_sNewline;     // "\r\n" (upstream default, FileNewlineChar = nlcWindows)

#endif
