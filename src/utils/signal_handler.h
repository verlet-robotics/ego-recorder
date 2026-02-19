#pragma once

// Signal handling for clean shutdown.
//
// Uses the POSIX sigwait() pattern: signals are blocked in all threads
// and a dedicated thread waits on them synchronously. This avoids all
// async-signal-safety issues present with signal()/sigaction() handlers.
//
// Usage:
//   std::atomic<bool> shutdown{false};
//   setup_signal_handling(shutdown);   // call BEFORE spawning any threads
//   // ... start capture/write threads ...
//   while (!shutdown.load(std::memory_order_acquire)) {
//       // main loop
//   }

#include <atomic>

/// Block SIGTERM and SIGINT in all threads (must be called before creating
/// any threads so the blocked mask is inherited), then spawn a dedicated
/// sigwait thread that sets \p shutdown_flag when a signal is received.
///
/// The spawned thread is detached and runs until process exit.
void setup_signal_handling(std::atomic<bool>& shutdown_flag);
