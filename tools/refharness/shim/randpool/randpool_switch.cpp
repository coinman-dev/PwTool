// Defines RandomPool::GetInstance() for all units except RandomPool.cpp
// itself (which is compiled with GetInstance renamed to GetInstance_real).
// Normally it returns the genuine upstream pool (real entropy). After
// DetRand_Enable() it returns a RandomPool subclass whose GetData()/GetByte()
// (virtual in RandomGenerator) draw from a caller-supplied deterministic
// generator, so that e.g. PasswDatabase::Save() output becomes reproducible.
#include <vcl.h>
#include "RandomPool.h"
#include "detrand.h"

RandomPool& pt_real_random_pool();

namespace {

std::vector<std::vector<word8>> s_log;

class DetRandomPool : public RandomPool
{
public:
  explicit DetRandomPool(RandomGenerator* pSrc)
    : RandomPool(CipherType::ChaCha20, {}, false), m_pSrc(pSrc)
  {}

  void GetData(void* pBuf, word32 lNumOfBytes) override
  {
    m_pSrc->GetData(pBuf, lNumOfBytes);
    const word8* p = reinterpret_cast<const word8*>(pBuf);
    s_log.emplace_back(p, p + lNumOfBytes);
  }

  word8 GetByte(void) override
  {
    word8 b;
    GetData(&b, 1);
    return b;
  }

private:
  RandomGenerator* m_pSrc;
};

std::unique_ptr<DetRandomPool> s_pDetPool;

}

void DetRand_Enable(RandomGenerator* pSrc)
{
  s_pDetPool = std::make_unique<DetRandomPool>(pSrc);
  s_log.clear();
}

bool DetRand_IsEnabled()
{
  return s_pDetPool != nullptr;
}

const std::vector<std::vector<word8>>& DetRand_Log()
{
  return s_log;
}

RandomPool& RandomPool::GetInstance(void)
{
  if (s_pDetPool)
    return *s_pDetPool;
  return pt_real_random_pool();
}
