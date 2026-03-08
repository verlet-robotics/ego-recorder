#include <gtest/gtest.h>
#include "threading/bounded_queue.h"

#include <thread>
#include <vector>
#include <chrono>

TEST(BoundedQueue, BasicPushPop) {
    BoundedQueue<int> q(4);
    q.push(1);
    q.push(2);
    q.push(3);

    auto v1 = q.pop();
    ASSERT_TRUE(v1.has_value());
    EXPECT_EQ(*v1, 1);

    auto v2 = q.pop();
    EXPECT_EQ(*v2, 2);

    auto v3 = q.pop();
    EXPECT_EQ(*v3, 3);
}

TEST(BoundedQueue, DropOldestOnOverflow) {
    BoundedQueue<int> q(2);
    q.push(1);
    q.push(2);
    q.push(3);  // Should drop 1
    q.push(4);  // Should drop 2

    EXPECT_EQ(q.dropped(), 2u);
    EXPECT_EQ(q.size(), 2u);

    auto v1 = q.pop();
    EXPECT_EQ(*v1, 3);
    auto v2 = q.pop();
    EXPECT_EQ(*v2, 4);
}

TEST(BoundedQueue, CloseAndDrain) {
    BoundedQueue<int> q(4);
    q.push(1);
    q.push(2);
    q.close();

    // Should still get remaining items
    auto v1 = q.pop();
    EXPECT_EQ(*v1, 1);
    auto v2 = q.pop();
    EXPECT_EQ(*v2, 2);

    // Now should get nullopt
    auto v3 = q.pop();
    EXPECT_FALSE(v3.has_value());
}

TEST(BoundedQueue, CloseUnblocksWaitingPop) {
    BoundedQueue<int> q(4);

    bool got_nullopt = false;
    std::thread consumer([&] {
        auto val = q.pop();  // This will block
        got_nullopt = !val.has_value();
    });

    // Give consumer time to block
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    q.close();
    consumer.join();

    EXPECT_TRUE(got_nullopt);
}

TEST(BoundedQueue, ProducerConsumerConcurrent) {
    BoundedQueue<int> q(4);
    const int N = 1000;
    std::vector<int> received;
    received.reserve(N);

    std::thread consumer([&] {
        while (true) {
            auto val = q.pop();
            if (!val) break;
            received.push_back(*val);
        }
    });

    for (int i = 0; i < N; i++) {
        q.push(i);
    }
    q.close();
    consumer.join();

    // Should receive all non-dropped items
    size_t total = received.size() + q.dropped();
    EXPECT_EQ(total, static_cast<size_t>(N));

    // Received items should be in order
    for (size_t i = 1; i < received.size(); i++) {
        EXPECT_GT(received[i], received[i - 1]);
    }
}

TEST(BoundedQueue, MoveOnlyTypes) {
    BoundedQueue<std::unique_ptr<int>> q(2);
    q.push(std::make_unique<int>(42));

    auto val = q.pop();
    ASSERT_TRUE(val.has_value());
    EXPECT_EQ(**val, 42);
}

TEST(BoundedQueue, SizeReportsCorrectly) {
    BoundedQueue<int> q(4);
    EXPECT_EQ(q.size(), 0u);

    q.push(1);
    EXPECT_EQ(q.size(), 1u);

    q.push(2);
    EXPECT_EQ(q.size(), 2u);

    q.pop();
    EXPECT_EQ(q.size(), 1u);
}

TEST(BoundedQueue, DroppedStartsAtZero) {
    BoundedQueue<int> q(4);
    EXPECT_EQ(q.dropped(), 0u);
}
