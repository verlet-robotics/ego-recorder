cmake_minimum_required(VERSION 3.16)
project(ego-recorder VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Allow user to specify a local turbojpeg prefix (for systems where
# libturbojpeg0-dev is not system-installed but the .deb has been extracted).
# Usage: cmake -DTURBOJPEG_LOCAL_PREFIX=/path/to/extracted/usr ..
if(DEFINED TURBOJPEG_LOCAL_PREFIX)
    list(PREPEND CMAKE_PREFIX_PATH "${TURBOJPEG_LOCAL_PREFIX}")
    list(PREPEND CMAKE_INCLUDE_PATH "${TURBOJPEG_LOCAL_PREFIX}/include")
    list(PREPEND CMAKE_LIBRARY_PATH "${TURBOJPEG_LOCAL_PREFIX}/lib/x86_64-linux-gnu"
                                    "${TURBOJPEG_LOCAL_PREFIX}/lib")
    # Make pkg-config look in the local prefix first
    set(ENV{PKG_CONFIG_PATH} "${TURBOJPEG_LOCAL_PREFIX}/lib/x86_64-linux-gnu/pkgconfig:$ENV{PKG_CONFIG_PATH}")
    message(STATUS "Using local TurboJPEG prefix: ${TURBOJPEG_LOCAL_PREFIX}")
endif()

# Allow user to specify the RealSense2 cmake prefix
if(DEFINED REALSENSE2_CMAKE_PREFIX)
    list(PREPEND CMAKE_PREFIX_PATH "${REALSENSE2_CMAKE_PREFIX}")
endif()

# ---- Dependencies ----

# librealsense2 (typically from ROS or system install)
find_package(realsense2 REQUIRED)

# PkgConfig for turbojpeg and zstd
find_package(PkgConfig REQUIRED)
pkg_check_modules(TURBOJPEG REQUIRED IMPORTED_TARGET libturbojpeg)
pkg_check_modules(ZSTD REQUIRED IMPORTED_TARGET libzstd)

# cxxopts via FetchContent
include(FetchContent)
FetchContent_Declare(
    cxxopts
    GIT_REPOSITORY https://github.com/jarro2783/cxxopts.git
    GIT_TAG        v3.3.1
)
FetchContent_MakeAvailable(cxxopts)

# Optional systemd support
option(WITH_SYSTEMD "Enable systemd sd_notify watchdog support" ON)
if(WITH_SYSTEMD)
    pkg_check_modules(SYSTEMD IMPORTED_TARGET libsystemd)
    if(SYSTEMD_FOUND)
        add_compile_definitions(HAVE_SYSTEMD)
        message(STATUS "systemd support: enabled")
    else()
        message(STATUS "systemd support: disabled (libsystemd not found)")
    endif()
endif()

# ---- Sources ----

add_executable(ego-recorder
    src/main.cpp
    src/capture/pipeline.cpp
    src/compression/jpeg_compressor.cpp
    src/compression/zstd_compressor.cpp
    src/storage/file_writer.cpp
    src/utils/signal_handler.cpp
    src/utils/stats.cpp
)

target_include_directories(ego-recorder PRIVATE src)

target_link_libraries(ego-recorder PRIVATE
    realsense2::realsense2
    PkgConfig::TURBOJPEG
    PkgConfig::ZSTD
    cxxopts::cxxopts
    pthread
)

if(WITH_SYSTEMD AND SYSTEMD_FOUND)
    target_link_libraries(ego-recorder PRIVATE PkgConfig::SYSTEMD)
endif()

install(TARGETS ego-recorder DESTINATION bin)
