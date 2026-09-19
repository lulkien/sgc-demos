cmake_minimum_required(VERSION 3.16)
project(adguard_dashboard C)

# AdGuard Home statistics dashboard on LVGL, rendered through a @sgc DRM lease.
#
# Everything this build needs is either committed here or fetched by git ref:
# lvgl and libsgc (libsgc-c) come from their repositories by ref, cJSON and
# tomlc99 are git submodules under third_party/, and lv_conf.h is committed.
# HTTP is libcurl. No sibling checkout, no config-generation step.

set(LVGL_REPO  "https://github.com/lulkien/lvgl.git" CACHE STRING "lvgl fork repository")
set(LVGL_REF   "dev/v9.6-sgc"                        CACHE STRING "lvgl fork branch or tag")
set(LVGL_ARCHIVE_URL ""                              CACHE STRING "fetch this archive instead of cloning")
set(SGC_REPO  "https://github.com/lulkien/libsgc-c.git"    CACHE STRING "libsgc-c repository")
set(SGC_REF   "800762c79e2ab45a234f231477b69b36b6c380a4"   CACHE STRING "libsgc-c commit, tag or branch")
set(SGC_ARCHIVE "" CACHE FILEPATH "prebuilt libsgc (libsgc.a, or libsgc.so for a shared build) - skips fetching and building the crate")
set(SGC_INCLUDE_DIR "" CACHE PATH "libsgc headers when using a prebuilt lib (default /usr/include). A shared libsgc.so is loaded at run time: install it on the target (e.g. /usr/local/lib/libsgc.so.0 + ldconfig) or set LD_LIBRARY_PATH")

# The LVGL config for this build. It is committed as lv_conf.h; pass another
# path (-DLV_BUILD_CONF_PATH=...) to build a variant, e.g. the GBM flavor.
if(NOT EXISTS ${LV_BUILD_CONF_PATH})
    message(FATAL_ERROR "LVGL config not found: ${LV_BUILD_CONF_PATH} - the repo's lv_conf.h is "
                        "committed; regenerate it with scripts/gen-lv-conf.sh --sgc --no-input "
                        "<lvgl checkout> lv_conf.h")
endif()

file(READ ${LV_BUILD_CONF_PATH} LV_CONF_TEXT)
if(LV_CONF_TEXT MATCHES "LV_USE_SGC +1")
    set(CONSUMER_SGC ON)
endif()

find_package(PkgConfig REQUIRED)
pkg_check_modules(CURL libcurl)
if(NOT CURL_FOUND)
    # Fallback for a host without libcurl's dev package: the public headers are
    # vendored under third_party/curl/include (curl 8.14.1, matching the runtime)
    # and the library is passed by path - typically the device's own
    # libcurl.so.4, copied off the board. The linker records its soname, so the
    # binary runs on the device unchanged.
    set(CURL_LIBRARY "" CACHE FILEPATH "libcurl.so to link when pkg-config has no libcurl")
    if(NOT CURL_LIBRARY)
        message(FATAL_ERROR "libcurl not found: install libcurl4-gnutls-dev[:arm64], or pass "
                            "-DCURL_LIBRARY=<path to libcurl.so> (headers are vendored in third_party/curl/include)")
    endif()
    if(NOT EXISTS ${CURL_LIBRARY})
        message(FATAL_ERROR "CURL_LIBRARY does not exist: ${CURL_LIBRARY}")
    endif()
    set(CURL_INCLUDE_DIRS ${CMAKE_CURRENT_SOURCE_DIR}/third_party/curl/include)
    set(CURL_LIBRARIES ${CURL_LIBRARY})
    # The device's libcurl.so.4 pulls in openssl/nghttp2/ssh2/... which are not
    # on the build host: let the linker leave those shared-library symbols for
    # the device's runtime linker to resolve.
    set(CURL_LINK_OPTIONS -Wl,--allow-shlib-undefined)
    message(STATUS "curl: vendored headers + ${CURL_LIBRARY}")
endif()
pkg_check_modules(DRM REQUIRED libdrm)

# lv_conf.h decides the scanout buffer flavor: GBM/DMA-BUF needs libgbm linked
# (lv_linux_drm.c calls gbm_bo_create/gbm_bo_map directly).
if(LV_CONF_TEXT MATCHES "LV_USE_LINUX_DRM_GBM_BUFFERS +1")
    pkg_check_modules(GBM REQUIRED libgbm)
    set(CONSUMER_GBM ON)
endif()

include(FetchContent)
if(POLICY CMP0169)
    # libsgc-c has no CMakeLists.txt, so it is fetched with FetchContent_Populate
    # rather than FetchContent_MakeAvailable; opt in explicitly on CMake >= 3.30.
    cmake_policy(SET CMP0169 OLD)
endif()
if(LVGL_ARCHIVE_URL)
    message(STATUS "dashboard: fetching ${LVGL_ARCHIVE_URL}")
    FetchContent_Declare(lvgl URL ${LVGL_ARCHIVE_URL})
else()
    FetchContent_Declare(lvgl
        GIT_REPOSITORY ${LVGL_REPO}
        GIT_TAG        ${LVGL_REF}
        GIT_SHALLOW    TRUE)
endif()

set(LV_BUILD_CONF_PATH ${CMAKE_CURRENT_SOURCE_DIR}/lv_conf.h CACHE PATH
    "lv_conf.h for this build; pass another conf to build a variant (e.g. GBM)")
set(CONFIG_LV_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(CONFIG_LV_BUILD_DEMOS    OFF CACHE BOOL "" FORCE)
set(CONFIG_LV_USE_THORVG_INTERNAL OFF CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(lvgl)

# headers for the lvgl targets (the drivers are compiled inside liblvgl)
target_include_directories(lvgl SYSTEM PUBLIC ${DRM_INCLUDE_DIRS})
if(CONSUMER_GBM)
    target_include_directories(lvgl SYSTEM PUBLIC ${GBM_INCLUDE_DIRS})
    message(STATUS "dashboard: GBM/DMA-BUF scanout buffers (libgbm ${GBM_VERSION})")
endif()
if(CONSUMER_SGC)
    if(CMAKE_SYSTEM_PROCESSOR MATCHES "aarch64|arm64")
        set(_sgc_arch aarch64)
    else()
        set(_sgc_arch x86_64)
    endif()
    set(_sgc_vendor ${CMAKE_CURRENT_SOURCE_DIR}/third_party/libsgc)

    if(SGC_ARCHIVE)
        # Escape hatch: link an archive built elsewhere, e.g. /usr/lib/libsgc.a
        # from the libsgc-dev package.
        if(NOT SGC_INCLUDE_DIR)
            set(SGC_INCLUDE_DIR /usr/include)
        endif()
        message(STATUS "dashboard: rendering through @sgc (prebuilt ${SGC_ARCHIVE})")
    elseif(EXISTS ${_sgc_vendor}/lib/${_sgc_arch}/libsgc.a)
        # Vendored archive: a prebuilt libsgc for this architecture, committed
        # under third_party/libsgc (see its README). The normal build therefore
        # needs neither cargo nor the network, and linking a fixed archive also
        # pins libsgc-rs, which libsgc-c does not lock.
        set(SGC_ARCHIVE ${_sgc_vendor}/lib/${_sgc_arch}/libsgc.a)
        set(SGC_INCLUDE_DIR ${_sgc_vendor}/include)
        file(READ ${_sgc_vendor}/REF _sgc_vendor_ref)
        string(STRIP "${_sgc_vendor_ref}" _sgc_vendor_ref)
        if(NOT _sgc_vendor_ref STREQUAL SGC_REF)
            message(WARNING "vendored libsgc is ${_sgc_vendor_ref} but SGC_REF is ${SGC_REF} - "
                            "run scripts/vendor-libsgc.sh to refresh")
        endif()
        message(STATUS "dashboard: rendering through @sgc (vendored ${_sgc_vendor_ref})")
    else()
        # Consume libsgc BY GIT REF, the same rule as lvgl: fetch the crate and
        # build its archive here, so no sibling checkout is involved. Needs
        # cargo and the rust target for the chosen architecture.
        # (FetchContent_Populate's single-argument form: fine while this project
        # states cmake_minimum_required below 3.30; libsgc-c has no CMakeLists
        # for FetchContent_MakeAvailable to consume.)
        FetchContent_Declare(libsgc_c
            GIT_REPOSITORY ${SGC_REPO}
            GIT_TAG        ${SGC_REF})
        FetchContent_Populate(libsgc_c)

        set(SGC_INCLUDE_DIR ${libsgc_c_SOURCE_DIR}/include)
        if(CMAKE_SYSTEM_PROCESSOR MATCHES "aarch64|arm64")
            set(SGC_ARCHIVE ${libsgc_c_SOURCE_DIR}/target/aarch64-unknown-linux-gnu/release/libsgc.a)
            set(_sgc_run ${CMAKE_COMMAND} -E env
                CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
                cargo build --release --target aarch64-unknown-linux-gnu)
        else()
            set(SGC_ARCHIVE ${libsgc_c_SOURCE_DIR}/target/release/libsgc.a)
            set(_sgc_run cargo build --release)
        endif()

        message(STATUS "dashboard: rendering through @sgc (building ${SGC_REF})")
        execute_process(COMMAND ${_sgc_run}
                        WORKING_DIRECTORY ${libsgc_c_SOURCE_DIR}
                        RESULT_VARIABLE _sgc_rc)
        if(NOT _sgc_rc EQUAL 0)
            message(FATAL_ERROR "cargo build failed for libsgc-c (${SGC_REF}) in ${libsgc_c_SOURCE_DIR}")
        endif()
    endif()

    if(NOT EXISTS ${SGC_ARCHIVE})
        message(FATAL_ERROR "libsgc not found at ${SGC_ARCHIVE}")
    endif()
    target_include_directories(lvgl SYSTEM PUBLIC ${SGC_INCLUDE_DIR})
    message(STATUS "dashboard: libsgc archive ${SGC_ARCHIVE}")
else()
    message(STATUS "dashboard: direct DRM display (LV_USE_SGC is off)")
endif()

add_executable(adguard_dashboard
    src/main.c
    src/config.c
    src/agh.c
    src/ui.c
    third_party/cJSON/cJSON.c
    third_party/tomlc99/toml.c)

target_include_directories(adguard_dashboard PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${CMAKE_CURRENT_SOURCE_DIR}/third_party/cJSON
    ${CMAKE_CURRENT_SOURCE_DIR}/third_party/tomlc99
    ${CURL_INCLUDE_DIRS}
    lvgl)

target_link_libraries(adguard_dashboard PRIVATE lvgl ${DRM_LIBRARIES} ${CURL_LIBRARIES} m)
if(CONSUMER_GBM)
    target_link_libraries(adguard_dashboard PRIVATE ${GBM_LIBRARIES})
endif()
if(CURL_LINK_OPTIONS)
    target_link_options(adguard_dashboard PRIVATE ${CURL_LINK_OPTIONS})
endif()
if(CONSUMER_SGC)
    target_link_libraries(adguard_dashboard PRIVATE ${SGC_ARCHIVE} pthread dl)
endif()
