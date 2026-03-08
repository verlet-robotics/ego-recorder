#include <gtest/gtest.h>
#include "utils/stats.h"

#include <thread>
#include <chrono>

TEST(Stats, InitialValuesAreZero) {
    Stats stats;
    EXPECT_EQ(stats.captured(), 0u);
    EXPECT_EQ(stats.written(), 0u);
    EXPECT_EQ(stats.dropped(), 0u);
    EXPECT_EQ(stats.total_bytes(), 0u);
    EXPECT_FALSE(stats.is_recording());
}

TEST(Stats, FrameCapturedIncrements) {
    Stats stats;
    stats.frame_captured();
    stats.frame_captured();
    stats.frame_captured();
    EXPECT_EQ(stats.captured(), 3u);
}

TEST(Stats, FrameWrittenIncrements) {
    Stats stats;
    stats.frame_written();
    stats.frame_written();
    EXPECT_EQ(stats.written(), 2u);
}

TEST(Stats, FramesDroppedAccumulates) {
    Stats stats;
    stats.frames_dropped(5);
    stats.frames_dropped(3);
    EXPECT_EQ(stats.dropped(), 8u);
}

TEST(Stats, BytesWrittenAccumulates) {
    Stats stats;
    stats.bytes_written(1024);
    stats.bytes_written(2048);
    EXPECT_EQ(stats.total_bytes(), 3072u);
}

TEST(Stats, ElapsedSecondsIsPositive) {
    Stats stats;
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
    EXPECT_GT(stats.elapsed_seconds(), 0.0);
}

TEST(Stats, RecordingElapsedTracksRecordingOnly) {
    Stats stats;
    EXPECT_NEAR(stats.recording_elapsed_seconds(), 0.0, 0.001);

    stats.recording_started();
    EXPECT_TRUE(stats.is_recording());

    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    double during = stats.recording_elapsed_seconds();
    EXPECT_GT(during, 0.03);

    stats.recording_stopped();
    EXPECT_FALSE(stats.is_recording());

    double after = stats.recording_elapsed_seconds();
    EXPECT_GT(after, 0.03);

    // After stopping, recording elapsed should not increase
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    double later = stats.recording_elapsed_seconds();
    EXPECT_NEAR(later, after, 0.005);
}

TEST(Stats, RecordingElapsedAccumulatesAcrossSessions) {
    Stats stats;

    stats.recording_started();
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    stats.recording_stopped();
    double first = stats.recording_elapsed_seconds();

    stats.recording_started();
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    stats.recording_stopped();
    double second = stats.recording_elapsed_seconds();

    EXPECT_GT(second, first);
    EXPECT_GT(second, 0.08);
}

TEST(Stats, CaptureFpsCalculation) {
    Stats stats;
    // Sleep briefly so elapsed_seconds() exceeds the 1µs guard clause
    std::this_thread::sleep_for(std::chrono::milliseconds(2));
    for (int i = 0; i < 100; i++) {
        stats.frame_captured();
    }
    // FPS = captured / elapsed. With tiny elapsed, FPS should be very high
    double fps = stats.capture_fps();
    EXPECT_GT(fps, 0.0);
}

TEST(Stats, WriteFpsUsesRecordingElapsed) {
    Stats stats;
    stats.recording_started();
    for (int i = 0; i < 100; i++) {
        stats.frame_written();
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(50));

    double wfps = stats.write_fps();
    EXPECT_GT(wfps, 0.0);

    stats.recording_stopped();
}

TEST(Stats, SummaryRecordingFormat) {
    Stats stats;
    stats.recording_started();
    stats.frame_captured();
    stats.frame_written();
    stats.bytes_written(1024 * 1024);

    std::string summary = stats.summary();
    EXPECT_NE(summary.find("REC"), std::string::npos)
        << "Summary during recording should contain 'REC': " << summary;
}

TEST(Stats, SummaryIdleFormat) {
    Stats stats;
    stats.frame_captured();

    std::string summary = stats.summary();
    EXPECT_NE(summary.find("Idle"), std::string::npos)
        << "Summary when idle should contain 'Idle': " << summary;
}

TEST(Stats, SummaryIdleWithPreviousRecording) {
    Stats stats;
    stats.recording_started();
    stats.frame_written();
    stats.bytes_written(1000);
    stats.recording_stopped();

    std::string summary = stats.summary();
    EXPECT_NE(summary.find("Last rec"), std::string::npos)
        << "Idle summary with previous recording should mention 'Last rec': " << summary;
}

TEST(Stats, ThreadSafety) {
    Stats stats;
    const int N = 10000;

    std::thread t1([&] {
        for (int i = 0; i < N; i++) stats.frame_captured();
    });
    std::thread t2([&] {
        for (int i = 0; i < N; i++) stats.frame_written();
    });
    std::thread t3([&] {
        for (int i = 0; i < N; i++) stats.bytes_written(100);
    });

    t1.join();
    t2.join();
    t3.join();

    EXPECT_EQ(stats.captured(), static_cast<uint64_t>(N));
    EXPECT_EQ(stats.written(), static_cast<uint64_t>(N));
    EXPECT_EQ(stats.total_bytes(), static_cast<uint64_t>(N * 100));
}
