#pragma once

// audio_alert -- audible countdown and beep for headless recording.
//
// Generates PCM sine wave tones and plays them via aplay (ALSA).
// Falls back to console bell (\a) if aplay is not available.

#include <atomic>

/// Play a short beep tone via ALSA (aplay).
/// Blocks until playback completes. Falls back to console bell if unavailable.
void play_beep(int frequency_hz = 800, int duration_ms = 150);

/// Play a recording countdown: 3 short beeps at 1-second intervals,
/// then a higher-pitched "go" beep. Blocks for ~3 seconds.
/// Checks shutdown_flag between beeps to allow early abort via Ctrl+C.
void play_countdown(const std::atomic<bool>& shutdown_flag);

/// Speak a short phrase via espeak-ng (or espeak fallback).
/// Blocks until speech completes. Falls back to beep if unavailable.
void play_speech(const char* text);
