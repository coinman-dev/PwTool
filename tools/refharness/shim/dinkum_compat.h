// Force-included (-include) into PasswGen.cpp and zxcvbn.cpp.
//
// The original PwTech.exe (RAD Studio 37.0, bcc64) links the Dinkumware C and
// C++ libraries statically; the harness uses llvm-mingw's libc++ and wine's
// ucrtbase. Where that changes results of the compiled upstream code, this
// header substitutes Dinkumware-compatible behaviour:
//
//  * log(): Dinkumware _Log(x, 0), reconstructed from the original binary
//    (dinkum_compat.cpp). Calls with a constant argument (e.g. log(2.0)) are
//    folded at compile time, as the original compiler (also clang) did - the
//    LOG2 constant in CalcPermSetEntropy is stored as 0x3FE62E42FEFA39EF.
//    pow()/exp() are NOT replaced (only used by zxcvbn's spatial matches).
//  * std::sort() (only call: PasswGen.cpp ParseCharSet, sorting subsets by
//    frequency): Dinkumware sorts ranges of <= 32 elements by insertion sort,
//    i.e. stably; libc++ partitions (unstably) already above 6 elements for
//    non-trivially-copyable types. Equal frequencies would then come out in a
//    different order. Replaced by a stable sort for <= 32 elements.
#pragma once
#include "vclshim.h"   // pull in all library headers before defining macros
#include <cmath>
#include <algorithm>
#include <optional>
#include <unordered_set>

double pt_dinkum_log(double x);

namespace std {
using ::pt_dinkum_log;

template<class RanIt, class Pred>
void pt_dinkum_sort(RanIt first, RanIt last, Pred pred)
{
  if (last - first <= 32)
    std::stable_sort(first, last, pred);   // == Dinkumware insertion sort
  else
    std::sort(first, last, pred);          // (not reproduced for > 32)
}
}

#define log(x) (__builtin_constant_p(x) ? ::log(x) : pt_dinkum_log(x))
#define sort(a, b, c) pt_dinkum_sort(a, b, c)
