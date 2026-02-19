#pragma once

// IPresenter -- Strategy interface for presentation layer.
//
// Defines the lifecycle contract shared by GuiPresenter (Dear ImGui) and
// HeadlessPresenter (systemd watchdog).  The recording engine calls these
// methods; the concrete presenter handles rendering, logging or sd_notify.
//
// Lifecycle sequence called by main.cpp:
//   1. start()      -- once, after camera + writer are initialized
//   2. tick()       -- in main loop; returns false when presenter wants to quit
//   3. shutdown()   -- once, on shutdown (signal or tick() returning false)
//
// Camera disconnect events are delivered out-of-band via on_camera_disconnect()
// and on_camera_reconnect().  Stats are pushed via update_stats() each loop.

#include "utils/stats.h"

class IPresenter {
public:
    virtual ~IPresenter() = default;

    /// Called once after camera and writer are initialized.
    /// Returns false if the presenter cannot start (e.g. no display server for GUI).
    virtual bool start() = 0;

    /// Called each iteration of the main loop.
    /// GUI:      renders one frame, polls window events.
    /// Headless: pings watchdog, checks disk space.
    /// Returns false when the presenter wants to quit (e.g. window closed, signal).
    virtual bool tick() = 0;

    /// Called on shutdown.  Flush final status, destroy resources.
    virtual void shutdown() = 0;

    /// Notifies the presenter that the camera has disconnected.
    virtual void on_camera_disconnect() = 0;

    /// Notifies the presenter that the camera has reconnected.
    virtual void on_camera_reconnect() = 0;

    /// Passes the latest recording stats for display or reporting.
    virtual void update_stats(const Stats& stats) = 0;
};
