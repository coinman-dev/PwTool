// Harness replacement for upstream src/main/Configuration.h (GUI-bound).
// Only the fields read by the compiled core units are declared; defaults are
// the upstream defaults.
#ifndef ConfigurationH
#define ConfigurationH
#include "UnicodeUtil.h"

struct Configuration {
  CharacterEncoding FileEncoding = ceUtf8;   // upstream default (Main.cpp:1118)
};

#endif
