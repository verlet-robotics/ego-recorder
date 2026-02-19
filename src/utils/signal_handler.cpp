// Signal handler -- POSIX sigwait()-based clean shutdown.
//
// Why sigwait and not signal()/sigaction()?
// In a multi-threaded C++ program, async signal handlers have severe
// restrictions: they can only call async-signal-safe functions (no printf,
// no C++ runtime). The sigwait pattern sidesteps this entirely: signals
// are blocked in all threads, and a dedicated thread waits on them
// synchronously, making the handler code completely unrestricted.

#include "utils/signal_handler.h"

#include <signal.h>   // sigset_t, sigemptyset, sigaddset, sigwait
#include <pthread.h>  // pthread_sigmask
#include <cstdio>     // fprintf
#include <thread>     // std::thread
#include <atomic>     // std::atomic<bool>

void setup_signal_handling(std::atomic<bool>& shutdown_flag) {
    // Build the set of signals to block and wait on.
    sigset_t sigset;
    sigemptyset(&sigset);
    sigaddset(&sigset, SIGTERM);
    sigaddset(&sigset, SIGINT);

    // Block SIGTERM and SIGINT in the calling thread.
    // Any threads created after this call inherit the blocked mask,
    // so they will never receive these signals asynchronously.
    pthread_sigmask(SIG_BLOCK, &sigset, nullptr);

    // Spawn a dedicated thread that calls sigwait().
    // The thread captures shutdown_flag by reference -- it is owned by main()
    // and outlives the detached thread (process exits when main() returns).
    std::thread([sigset, &shutdown_flag]() mutable {
        int sig = 0;
        // sigwait blocks until one of the signals in sigset is delivered.
        // This is safe and unrestricted -- no async-signal-safety concerns.
        int rc = sigwait(&sigset, &sig);
        if (rc == 0) {
            std::fprintf(stderr, "\nReceived signal %d, shutting down...\n", sig);
        } else {
            std::fprintf(stderr, "\nsigwait returned error %d, shutting down...\n", rc);
        }
        shutdown_flag.store(true, std::memory_order_release);
    }).detach();
}
