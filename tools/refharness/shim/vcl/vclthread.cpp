// TThread / TEvent shim (see vclshim.h)
#include "vclshim.h"

THandleObject::~THandleObject()
{
  if (m_hHandle)
    CloseHandle(m_hHandle);
}

TEvent::TEvent(void*, bool blManualReset, bool blInitialState,
  const UnicodeString& sName, bool)
{
  m_hHandle = CreateEventW(nullptr, blManualReset, blInitialState,
    sName.IsEmpty() ? nullptr : sName.c_str());
}

TWaitResult TEvent::WaitFor(Cardinal timeout)
{
  switch (WaitForSingleObject(m_hHandle, timeout)) {
  case WAIT_OBJECT_0: return wrSignaled;
  case WAIT_TIMEOUT: return wrTimeout;
  case WAIT_ABANDONED: return wrAbandoned;
  default:
    m_nLastError = GetLastError();
    return wrError;
  }
}

void TEvent::SetEvent() { ::SetEvent(m_hHandle); }
void TEvent::ResetEvent() { ::ResetEvent(m_hHandle); }

DWORD WINAPI TThread::ThreadProc(LPVOID p)
{
  TThread* pThread = static_cast<TThread*>(p);
  try {
    pThread->Execute();
  }
  catch (...) {
  }
  pThread->m_blFinished = true;
  return 0;
}

TThread::TThread(bool blCreateSuspended)
{
  m_hThread = CreateThread(nullptr, 0, ThreadProc, this, CREATE_SUSPENDED, nullptr);
  if (!blCreateSuspended)
    Start();
}

TThread::~TThread()
{
  if (m_hThread) {
    if (!m_blFinished) {
      Terminate();
      ResumeThread(m_hThread);
      WaitForSingleObject(m_hThread, INFINITE);
    }
    CloseHandle(m_hThread);
  }
}

void TThread::Start()
{
  ResumeThread(m_hThread);
}

Cardinal TThread::WaitFor()
{
  WaitForSingleObject(m_hThread, INFINITE);
  return 0;
}
