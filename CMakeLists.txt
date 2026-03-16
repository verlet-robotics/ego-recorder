cmake_minimum_required(VERSION 3.16)
project(ego-recorder VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Allow user to specify a local turbojpeg prefix (for systems where
# libturbojpeg0-dev is not system-installed but the .deb has been extracted).
# Usage: cmake -DTURBOJPEG_LOCAL_PREFIX=/path/to/extracted/usr ..
#
# The patched pkg-config file /tmp/turbojpeg-pkgconfig/libturbojpeg.pc should
# also exist with correct prefix= set to TURBOJPEG_LOCAL_PREFIX.
if(DEFINED TURBOJPEG_LOCAL_PREFIX)
    # Point pkg-config to a patched .pc file directory with correct absolute paths.
    # The patched file is typically at /tmp/turbojpeg-pkgconfig/libturbojpeg.pc
    # (created alongside the extracted deb) or we generate it here from scratch.
    set(TURBOJPEG_PKGCONFIG_DIR "${CMAKE_BINARY_DIR}/turbojpeg-pkgconfig")
    file(MAKE_DIRECTORY "${TURBOJPEG_PKGCONFIG_DIR}")
    set(TURBOJPEG_LIBDIR "${TURBOJPEG_LOCAL_PREFIX}/lib/x86_64-linux-gnu")
    # Fallback: if arch lib dir doesn't exist, try plain lib/
    if(NOT EXISTS "${TURBOJPEG_LIBDIR}")
        set(TURBOJPEG_LIBDIR "${TURBOJPEG_LOCAL_PREFIX}/lib")
    endif()
    file(WRITE "${TURBOJPEG_PKGCONFIG_DIR}/libturbojpeg.pc"
        "prefix=${TURBOJPEG_LOCAL_PREFIX}\n"
        "libdir=${TURBOJPEG_LIBDIR}\n"
        "includedir=${TURBOJPEG_LOCAL_PREFIX}/include\n"
        "\n"
        "Name: libturbojpeg\n"
        "Description: TurboJPEG (local extract)\n"
        "Version: 2.1.5\n"
        "Libs: -L${TURBOJPEG_LIBDIR} -lturbojpeg\n"
        "Cflags: -I${TURBOJPEG_LOCAL_PREFIX}/include\n"
    )
    set(ENV{PKG_CONFIG_PATH} "${TURBOJPEG_PKGCONFIG_DIR}:$ENV{PKG_CONFIG_PATH}")
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

# toml++ via FetchContent (always built -- both GUI and headless need config)
FetchContent_Declare(
    tomlplusplus
    GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
    GIT_TAG        v3.4.0
)
FetchContent_MakeAvailable(tomlplusplus)

# nlohmann/json for dataset manifest (dataset.json read/write)
FetchContent_Declare(
    nlohmann_json
    GIT_REPOSITORY https://github.com/nlohmann/json.git
    GIT_TAG        v3.11.3
    GIT_SHALLOW    TRUE
)
set(JSON_BuildTests OFF CACHE INTERNAL "")
FetchContent_MakeAvailable(nlohmann_json)

# Zdepth depth compressor via FetchContent
# CRITICAL: Create a 'zstd' INTERFACE target BEFORE fetching Zdepth so that
# Zdepth's CMakeLists.txt (which checks `if(NOT TARGET zstd)`) skips its
# bundled zstd copy and uses our system libzstd instead. This prevents
# duplicate symbol linker errors. We use INTERFACE (not ALIAS) because
# Zdepth's CMake also does `export(TARGETS zdepth zstd ...)` which requires
# a real (non-ALIAS) target.
if(NOT TARGET zstd)
    add_library(zstd INTERFACE)
    target_link_libraries(zstd INTERFACE PkgConfig::ZSTD)
endif()
FetchContent_Declare(
    zdepth
    GIT_REPOSITORY https://github.com/catid/Zdepth.git
    GIT_TAG        master
)
# Prevent Zdepth's install(EXPORT) from failing because our 'zstd' interface
# target is not in its export set. We add our zstd target to the export set
# that Zdepth's CMake expects (it does `install(EXPORT zdepth ...)`).
install(TARGETS zstd EXPORT zdepth)
FetchContent_MakeAvailable(zdepth)

# FFmpeg via pkg-config (for H.264 encoding)
pkg_check_modules(AVCODEC REQUIRED IMPORTED_TARGET libavcodec)
pkg_check_modules(AVUTIL REQUIRED IMPORTED_TARGET libavutil)
pkg_check_modules(SWSCALE REQUIRED IMPORTED_TARGET libswscale)

# Optional Python extension module
option(WITH_PYTHON "Build Python extension module" ON)
if(WITH_PYTHON)
    # pybind11 via FetchContent
    FetchContent_Declare(
        pybind11
        GIT_REPOSITORY https://github.com/pybind/pybind11.git
        GIT_TAG        v2.13.6
    )
    FetchContent_MakeAvailable(pybind11)

    find_package(Python3 COMPONENTS Interpreter Development REQUIRED)

    # Build the egorec_reader Python extension module
    # ZdepthCompressor is our RAII wrapper (not the zdepth library itself),
    # so we compile its source directly into the extension module.
    pybind11_add_module(egorec_reader
        src/python/egorec_reader.cpp
        src/compression/zdepth_compressor.cpp
    )
    target_include_directories(egorec_reader PRIVATE src)
    target_link_libraries(egorec_reader PRIVATE
        zdepth
        PkgConfig::ZSTD
        PkgConfig::AVCODEC
        PkgConfig::AVUTIL
        PkgConfig::SWSCALE
    )

    # Place the .so in the build directory root for easy PYTHONPATH=build/ usage
    set_target_properties(egorec_reader PROPERTIES
        LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}"
    )

    message(STATUS "Python support: enabled (${Python3_EXECUTABLE})")
    message(STATUS "Python extension module: egorec_reader")
else()
    message(STATUS "Python support: disabled (-DWITH_PYTHON=OFF)")
endif()

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

# Optional Dear ImGui GUI mode
# Requires: libglfw3-dev, libgl-dev (or mesa-common-dev)
# Disable with: cmake -DWITH_GUI=OFF ..
option(WITH_GUI "Build Dear ImGui GUI mode" ON)
if(WITH_GUI)
    find_package(OpenGL REQUIRED)
    find_package(glfw3 REQUIRED)

    FetchContent_Declare(
        imgui
        GIT_REPOSITORY https://github.com/ocornut/imgui.git
        GIT_TAG        v1.92.6
    )
    FetchContent_MakeAvailable(imgui)

    # Build Dear ImGui as a static library with GLFW + OpenGL3 backend
    add_library(imgui_glfw_opengl3 STATIC
        ${imgui_SOURCE_DIR}/imgui.cpp
        ${imgui_SOURCE_DIR}/imgui_draw.cpp
        ${imgui_SOURCE_DIR}/imgui_tables.cpp
        ${imgui_SOURCE_DIR}/imgui_widgets.cpp
        ${imgui_SOURCE_DIR}/backends/imgui_impl_glfw.cpp
        ${imgui_SOURCE_DIR}/backends/imgui_impl_opengl3.cpp
        ${imgui_SOURCE_DIR}/misc/cpp/imgui_stdlib.cpp
    )
    target_include_directories(imgui_glfw_opengl3 PUBLIC
        ${imgui_SOURCE_DIR}
        ${imgui_SOURCE_DIR}/backends
        ${imgui_SOURCE_DIR}/misc/cpp
    )
    target_link_libraries(imgui_glfw_opengl3 PUBLIC glfw OpenGL::GL)

    add_compile_definitions(HAVE_GUI)
    message(STATUS "GUI support: enabled (Dear ImGui v1.92.6)")
else()
    message(STATUS "GUI support: disabled (-DWITH_GUI=OFF)")
endif()

# ---- Sources ----

add_executable(ego-recorder
    src/main.cpp
    src/capture/pipeline.cpp
    src/compression/jpeg_compressor.cpp
    src/compression/zstd_compressor.cpp
    src/compression/zdepth_compressor.cpp
    src/compression/h264_encoder.cpp
    src/storage/file_writer.cpp
    src/utils/signal_handler.cpp
    src/utils/stats.cpp
    src/utils/audio_alert.cpp
    src/config/config.cpp
    src/presenter/headless_presenter.cpp
    src/dataset/dataset_manifest.cpp
    src/dataset/dataset_commands.cpp
    src/utils/depth_colorizer.cpp
    src/presenter/preview_presenter.cpp
)

# gui_presenter.cpp is only compiled when WITH_GUI=ON (added in plan 02-02)
if(WITH_GUI)
    target_sources(ego-recorder PRIVATE src/presenter/gui_presenter.cpp)
endif()

target_include_directories(ego-recorder PRIVATE src)

target_link_libraries(ego-recorder PRIVATE
    realsense2::realsense2
    PkgConfig::TURBOJPEG
    PkgConfig::ZSTD
    zdepth
    PkgConfig::AVCODEC
    PkgConfig::AVUTIL
    PkgConfig::SWSCALE
    cxxopts::cxxopts
    tomlplusplus::tomlplusplus
    nlohmann_json::nlohmann_json
    pthread
)

if(WITH_SYSTEMD AND SYSTEMD_FOUND)
    target_link_libraries(ego-recorder PRIVATE PkgConfig::SYSTEMD)
endif()

if(WITH_GUI)
    target_link_libraries(ego-recorder PRIVATE imgui_glfw_opengl3)
endif()

install(TARGETS ego-recorder DESTINATION bin)

# ---- Rust export tools ----
option(WITH_RUST_EXPORT "Build Rust export tools (ego-convert)" ON)
if(WITH_RUST_EXPORT)
    add_custom_target(ego-convert ALL
        COMMAND cargo build --release --manifest-path ${CMAKE_SOURCE_DIR}/rust/Cargo.toml
        COMMENT "Building Rust export tools (ego-convert)")

    # Install the Rust binary alongside ego-recorder
    install(PROGRAMS ${CMAKE_SOURCE_DIR}/rust/target/release/ego-convert
            DESTINATION bin
            OPTIONAL)

    message(STATUS "Rust export tools: enabled")
else()
    message(STATUS "Rust export tools: disabled (-DWITH_RUST_EXPORT=OFF)")
endif()

# ---- Tests ----
option(BUILD_TESTS "Build unit tests" ON)
if(BUILD_TESTS)
    enable_testing()

    FetchContent_Declare(
        googletest
        GIT_REPOSITORY https://github.com/google/googletest.git
        GIT_TAG        v1.15.2
    )
    set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
    FetchContent_MakeAvailable(googletest)

    # Shared sources used by multiple test targets
    set(TEST_COMMON_SOURCES
        src/compression/jpeg_compressor.cpp
        src/compression/zstd_compressor.cpp
        src/compression/zdepth_compressor.cpp
        src/compression/h264_encoder.cpp
        src/storage/file_writer.cpp
        src/config/config.cpp
        src/utils/stats.cpp
    )

    add_executable(ego-recorder-tests
        tests/test_binary_format.cpp
        tests/test_file_writer.cpp
        tests/test_compression.cpp
        tests/test_bounded_queue.cpp
        tests/test_stats.cpp
        tests/test_config.cpp
        tests/test_integration.cpp
        tests/test_dataset_manifest.cpp
        src/dataset/dataset_manifest.cpp
        ${TEST_COMMON_SOURCES}
    )

    target_include_directories(ego-recorder-tests PRIVATE src)
    target_link_libraries(ego-recorder-tests PRIVATE
        gtest_main
        PkgConfig::TURBOJPEG
        PkgConfig::ZSTD
        zdepth
        PkgConfig::AVCODEC
        PkgConfig::AVUTIL
        PkgConfig::SWSCALE
        tomlplusplus::tomlplusplus
        nlohmann_json::nlohmann_json
        pthread
    )

    include(GoogleTest)
    gtest_discover_tests(ego-recorder-tests)

    message(STATUS "Tests: enabled")
else()
    message(STATUS "Tests: disabled (-DBUILD_TESTS=OFF)")
endif()
