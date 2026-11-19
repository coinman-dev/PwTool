// Force-included (-include) into every C++ translation unit of the harness.
// Provides the bits of the C++Builder dialect that clang/libc++ lack and that
// cannot be supplied by an ordinary header.
#pragma once

#ifndef NOMINMAX
#define NOMINMAX            // upstream uses std::min/std::max everywhere
#endif

#ifdef __cplusplus

#include <cstddef>
#include <cstring>
#include <cwchar>
#include <exception>
#include <string>
#include <ios>

// ---------------------------------------------------------------------------
// std::basic_string<word32> (w32string): libc++ >= 19 has no generic
// char_traits any more. C++Builder's Dinkumware library has it, so provide an
// equivalent specialisation (ordinal compare, like the generic template).
namespace std {
template<> struct char_traits<unsigned int> {
  using char_type = unsigned int;
  using int_type = unsigned int;
  using off_type = streamoff;
  using pos_type = streampos;
  using state_type = mbstate_t;
  static constexpr void assign(char_type& r, const char_type& a) noexcept { r = a; }
  static constexpr bool eq(char_type a, char_type b) noexcept { return a == b; }
  static constexpr bool lt(char_type a, char_type b) noexcept { return a < b; }
  static constexpr int compare(const char_type* s1, const char_type* s2, size_t n) {
    for (; n; --n, ++s1, ++s2) {
      if (lt(*s1, *s2)) return -1;
      if (lt(*s2, *s1)) return 1;
    }
    return 0;
  }
  static constexpr size_t length(const char_type* s) {
    size_t n = 0;
    while (s[n] != 0) ++n;
    return n;
  }
  static constexpr const char_type* find(const char_type* s, size_t n, const char_type& a) {
    for (; n; --n, ++s)
      if (*s == a) return s;
    return nullptr;
  }
  static char_type* move(char_type* s1, const char_type* s2, size_t n) {
    if (n) memmove(s1, s2, n * sizeof(char_type));
    return s1;
  }
  static char_type* copy(char_type* s1, const char_type* s2, size_t n) {
    if (n) memcpy(s1, s2, n * sizeof(char_type));
    return s1;
  }
  static char_type* assign(char_type* s, size_t n, char_type a) {
    for (size_t i = 0; i < n; i++) s[i] = a;
    return s;
  }
  static constexpr int_type not_eof(int_type c) noexcept { return eq_int_type(c, eof()) ? ~eof() : c; }
  static constexpr char_type to_char_type(int_type c) noexcept { return c; }
  static constexpr int_type to_int_type(char_type c) noexcept { return c; }
  static constexpr bool eq_int_type(int_type c1, int_type c2) noexcept { return c1 == c2; }
  static constexpr int_type eof() noexcept { return static_cast<int_type>(0xFFFFFFFF); }
};
}

// ---------------------------------------------------------------------------
// Borland "try { A } __finally { B }".
//
// Expands to
//   try { A } catch (...) { <stash exception> }
//   for (scope; once(); done()) { B }
// i.e. B runs on the normal path and on the exception path, and done()
// rethrows the stashed exception after B has completed. Limitation: if A
// leaves via `return`/`break`, B is NOT executed (none of the compiled
// upstream units do that inside a __finally-protected block).
namespace pt_finally {
std::exception_ptr& slot();
struct Scope {
  std::exception_ptr ex;
  bool first = true;
  Scope() : ex(slot()) { slot() = nullptr; }
  bool once() { bool r = first; first = false; return r; }
  void done() {
    if (ex) {
      std::exception_ptr e = ex;
      ex = nullptr;
      std::rethrow_exception(e);
    }
  }
};
}

#define __try try
// SEH filter blocks (only in TScriptingThread::Execute, never run by the
// harness): compile them as a catch clause that can never match
namespace pt_finally { struct NeverThrown {}; }
#define __except(filter) catch (const ::pt_finally::NeverThrown&)
// (GetExceptionCode is redefined in vcl/vclshim.h, after <windows.h>)
#define __finally \
  catch (...) { ::pt_finally::slot() = std::current_exception(); } \
  for (::pt_finally::Scope __pt_fs; __pt_fs.once(); __pt_fs.done())

#endif // __cplusplus
