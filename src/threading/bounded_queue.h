#pragma once

#include <condition_variable>
#include <mutex>
#include <optional>
#include <queue>

/// Thread-safe bounded queue with drop-oldest policy.
///
/// When the queue is full and a new item is pushed, the oldest item is silently
/// discarded and a drop counter is incremented. This models back-pressure for
/// a producer that must never block (e.g., a RealSense capture callback).
///
/// Supports close() semantics: after close(), pop() drains remaining items and
/// then returns std::nullopt so consumers can exit their loops cleanly.
template <typename T>
class BoundedQueue {
public:
    explicit BoundedQueue(size_t max_size) : max_size_(max_size) {}

    // Non-copyable, non-movable (owns mutex + condvar)
    BoundedQueue(const BoundedQueue&) = delete;
    BoundedQueue& operator=(const BoundedQueue&) = delete;
    BoundedQueue(BoundedQueue&&) = delete;
    BoundedQueue& operator=(BoundedQueue&&) = delete;

    /// Push an item. If full, the oldest item is discarded and dropped_ is incremented.
    void push(T item) {
        std::lock_guard<std::mutex> lock(mutex_);
        if (queue_.size() >= max_size_) {
            queue_.pop();
            ++dropped_;
        }
        queue_.push(std::move(item));
        not_empty_.notify_one();
    }

    /// Blocking pop. Returns nullopt only when the queue is both closed and empty.
    std::optional<T> pop() {
        std::unique_lock<std::mutex> lock(mutex_);
        not_empty_.wait(lock, [this] { return !queue_.empty() || closed_; });
        if (queue_.empty()) {
            return std::nullopt;
        }
        T item = std::move(queue_.front());
        queue_.pop();
        return item;
    }

    /// Signal consumers to exit after draining remaining items.
    void close() {
        std::lock_guard<std::mutex> lock(mutex_);
        closed_ = true;
        not_empty_.notify_all();
    }

    /// Number of items dropped due to queue being full.
    size_t dropped() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return dropped_;
    }

    /// Current number of items in the queue.
    size_t size() const {
        std::lock_guard<std::mutex> lock(mutex_);
        return queue_.size();
    }

private:
    std::queue<T>           queue_;
    mutable std::mutex      mutex_;
    std::condition_variable not_empty_;
    size_t                  max_size_;
    bool                    closed_{false};
    size_t                  dropped_{0};
};
