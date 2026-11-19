// (see dinkum_compat.h) Dinkumware _Log(x, 0) (natural logarithm) as linked into PwTech.exe 3.6.1,
// reconstructed from the disassembly (VA 0x94b2a0, constants 0xdeb530..0xdeb5c8).
// Compile without FP contraction.
#include <cmath>
#include <cstring>
#include <limits>

static double bitsToDouble(unsigned long long u)
{
  double d;
  memcpy(&d, &u, 8);
  return d;
}

double pt_dinkum_log(double x)
{
  if (x != x)
    return x;
  if (x == 0)
    return -std::numeric_limits<double>::infinity();
  if (x < 0)
    return std::numeric_limits<double>::quiet_NaN();
  if (x == std::numeric_limits<double>::infinity())
    return x;

  // _Dunscale: x = m * 2^e, m in [0.5, 1)  (frexp is exact)
  int e;
  double m = std::frexp(x, &e);
  if (m < bitsToDouble(0x3fe6a09e667f3bcdULL)) {   // sqrt(0.5)
    m = m + m;
    e--;
  }
  static const double C[7] = {
    bitsToDouble(0x3fc2f8d8bf29def7ULL), bitsToDouble(0x3fc3991accfeb3f4ULL),
    bitsToDouble(0x3fc7466fd968cb62ULL), bitsToDouble(0x3fcc71c4da12c9c6ULL),
    bitsToDouble(0x3fd24924948ae94aULL), bitsToDouble(0x3fd9999999975ff7ULL),
    bitsToDouble(0x3fe55555555555bcULL)
  };
  volatile double vm = m;
  const double mm = vm;
  const double f = -1.0 + mm;
  const double d = mm + 1.0;
  const double z = f / d;
  const double w = z * z;
  double p = C[0] * w;
  p = (p + C[1]) * w;
  p = (p + C[2]) * w;
  p = (p + C[3]) * w;
  p = (p + C[4]) * w;
  p = (p + C[5]) * w;
  p = (p + C[6]) * w;
  double t = f - p;
  t = t * z;
  const double k = static_cast<double>(e);
  const double lo = bitsToDouble(0xbe7f7d1cf79abc9eULL) * k;   // -ln2_lo * k
  t = t + lo;
  const double r1 = f - t;
  const double hi = k * bitsToDouble(0x3fe62e42c0000000ULL);   // ln2_hi * k
  return hi + r1;
}
