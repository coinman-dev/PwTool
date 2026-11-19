// Compiled with -DGetInstance=GetInstance_real (like upstream RandomPool.cpp),
// so RandomPool::GetInstance() below refers to the genuine upstream singleton.
#include <vcl.h>
#include "RandomPool.h"

RandomPool& pt_real_random_pool()
{
  return RandomPool::GetInstance();
}
