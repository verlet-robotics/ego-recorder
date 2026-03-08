// audio_alert.cpp -- audible countdown and beep for headless recording.
//
// Generates PCM sine wave tones and pipes raw S16_LE samples to aplay.
// pclose() blocks until aplay finishes playback, giving accurate timing.
// Falls back to console bell if aplay is not available.

#include "utils/audio_alert.h"

#include <chrono>
#include <cmath>
#include <cstdio>
#include <thread>
#include <vector>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

static void play_tone(int frequency_hz, int duration_ms) {
    constexpr int sample_rate = 44100;
    const int num_samples = sample_rate * duration_ms / 1000;
    std::vector<int16_t> samples(num_samples);

    for (int i = 0; i < num_samples; ++i) {
        double t = static_cast<double>(i) / sample_rate;

        // 5ms fade in/out to avoid clicks
        double envelope = 1.0;
        constexpr int fade = sample_rate * 5 / 1000;
        if (i < fade) {
            envelope = static_cast<double>(i) / fade;
        } else if (i > num_samples - fade) {
            envelope = static_cast<double>(num_samples - i) / fade;
        }

        samples[i] = static_cast<int16_t>(
            24000.0 * envelope * std::sin(2.0 * M_PI * frequency_hz * t));
    }

    // Play via aplay (ALSA) -- pclose blocks until playback completes.
    FILE* pipe = popen("aplay -t raw -f S16_LE -r 44100 -c 1 -q 2>/dev/null", "w");
    if (pipe) {
        fwrite(samples.data(), sizeof(int16_t), samples.size(), pipe);
        pclose(pipe);
        return;
    }

    // Fallback: console bell
    fprintf(stderr, "\a");
    fflush(stderr);
}

void play_beep(int frequency_hz, int duration_ms) {
    play_tone(frequency_hz, duration_ms);
}

void play_speech(const char* text) {
    // Try espeak-ng first, then espeak
    char cmd[256];
    snprintf(cmd, sizeof(cmd), "espeak-ng -s 160 '%s' 2>/dev/null", text);
    if (system(cmd) == 0) return;

    snprintf(cmd, sizeof(cmd), "espeak -s 160 '%s' 2>/dev/null", text);
    if (system(cmd) == 0) return;

    // Fallback: beep
    play_tone(800, 200);
}

void play_countdown(const std::atomic<bool>& shutdown_flag) {
    // 3 countdown beeps at 1-second intervals
    for (int i = 3; i > 0; --i) {
        if (shutdown_flag.load(std::memory_order_acquire)) return;

        fprintf(stderr, "[headless] Recording in %d...\n", i);
        play_tone(800, 150);

        // Sleep remainder of 1 second in small increments for responsive abort
        for (int ms = 0; ms < 850 && !shutdown_flag.load(); ms += 50) {
            std::this_thread::sleep_for(std::chrono::milliseconds(50));
        }
    }

    if (shutdown_flag.load(std::memory_order_acquire)) return;

    // "Go" beep -- higher pitch, longer duration
    fprintf(stderr, "[headless] GO!\n");
    play_tone(1200, 300);
}
