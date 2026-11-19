// Deterministic replacement for RandomPool::GetInstance() (harness only).
#ifndef DetRandH
#define DetRandH
#include <vector>
#include "RandomGenerator.h"

// From now on RandomPool::GetInstance() returns a RandomPool whose
// GetData()/GetByte() draw from pSrc (which must outlive all uses).
void DetRand_Enable(RandomGenerator* pSrc);
bool DetRand_IsEnabled();

// every GetData()/GetByte() request served by the deterministic pool, in order
const std::vector<std::vector<word8>>& DetRand_Log();

#endif
