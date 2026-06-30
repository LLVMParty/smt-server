#pragma once
/*
 * GCC-ism polyfills for building cadical, symfpu, and bitwuzla with MSVC.
 * Force-included (/FI) before any project header.
 */
#ifdef _MSC_VER

/* GCC's __PRETTY_FUNCTION__ -> MSVC's __FUNCSIG__ (full signature string). */
#ifndef __PRETTY_FUNCTION__
#define __PRETTY_FUNCTION__ __FUNCSIG__
#endif
/* cadical uses __attribute__ only for compile-time annotations
 * (format-string checking; a .preinit_array section in mobical which is not
 * built here). Dropping them is safe. */
#ifndef __attribute__
#define __attribute__(A)
#endif

#include <intrin.h>
#include <math.h>

#define __builtin_expect(c, v) (c)
#define __builtin_unreachable() __assume(0)

static __forceinline void
__builtin_prefetch(const void *p, int rw, int locality)
{
  (void)rw;
  (void)locality;
  _mm_prefetch((const char *)p, _MM_HINT_T0);
}

static __forceinline int
__builtin_clz(unsigned x)
{
  unsigned long r;
  _BitScanReverse(&r, x);
  return 31 - (int)r;
}

static __forceinline int
__builtin_clzll(unsigned long long x)
{
  unsigned long r;
  _BitScanReverse64(&r, x);
  return 63 - (int)r;
}

static __forceinline int
__builtin_ctz(unsigned x)
{
  unsigned long r;
  _BitScanForward(&r, x);
  return (int)r;
}

static __forceinline int
__builtin_ctzll(unsigned long long x)
{
  unsigned long r;
  _BitScanForward64(&r, x);
  return (int)r;
}

static __forceinline int
__builtin_popcount(unsigned x)
{
  return (int)__popcnt(x);
}

static __forceinline int
__builtin_popcountll(unsigned long long x)
{
  return (int)__popcnt64(x);
}

/* symfpu uses __builtin_fmaf for fused multiply-add; MSVC has C99 fmaf. */
static __forceinline float
__builtin_fmaf(float a, float b, float c)
{
  return fmaf(a, b, c);
}

#endif /* _MSC_VER */
