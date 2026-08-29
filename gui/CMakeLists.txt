# The Qt6 front end (F-004), over the C ABI (D-001).
#
# Cargo builds the bridge; CMake builds this and links it. Keeping the two
# build systems in their own lanes — rather than teaching one to drive the
# other's internals — is what makes `cargo test` and `cmake --build` both work
# on their own.

cmake_minimum_required(VERSION 3.16)
project(ade-gui LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)
qt_standard_project_setup()

set(ADE_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/..")

# Release by default: a debug build of the engine is roughly ten times slower,
# which is very visible when opening a corpus image.
set(CARGO_PROFILE "release")
set(CARGO_FLAG "--release")
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(CARGO_PROFILE "debug")
    set(CARGO_FLAG "")
endif()

set(ADE_LIB "${ADE_ROOT}/target/${CARGO_PROFILE}/libade.a")

# Always re-run cargo: it is incremental and knows far better than CMake
# whether anything changed.
add_custom_target(ade_bridge ALL
    COMMAND cargo build ${CARGO_FLAG} -p ade-bridge --manifest-path "${ADE_ROOT}/Cargo.toml"
    BYPRODUCTS "${ADE_LIB}"
    COMMENT "Building the ADE bridge (cargo)"
    VERBATIM
)

qt_add_executable(ade-gui
    src/main.cpp
    src/MainWindow.cpp
    src/ImageTree.cpp
    src/HexView.cpp
    src/MainWindow.h
    src/ImageTree.h
    src/HexView.h
    src/Image.h
)
add_dependencies(ade-gui ade_bridge)

target_include_directories(ade-gui PRIVATE "${ADE_ROOT}/bridge/include")
target_link_libraries(ade-gui PRIVATE Qt6::Widgets "${ADE_LIB}" pthread dl m)

# The engine is warning-clean under a far stricter set than this; the GUI
# should not be the place standards slip.
target_compile_options(ade-gui PRIVATE -Wall -Wextra)

# Headless tests (F-004). A GUI's *feel* needs a person; what it *shows* does
# not, and that is what these check.
find_package(Qt6 QUIET COMPONENTS Test)
if(Qt6Test_FOUND)
    enable_testing()
    qt_add_executable(ade-gui-tests
        tests/test_mainwindow.cpp
        src/MainWindow.cpp
        src/ImageTree.cpp
        src/HexView.cpp
        src/MainWindow.h
        src/ImageTree.h
        src/HexView.h
        src/Image.h
    )
    add_dependencies(ade-gui-tests ade_bridge)
    target_include_directories(ade-gui-tests PRIVATE "${ADE_ROOT}/bridge/include" src)
    # Generate the test image at build time with the engine's own fixture
    # tool. D-010 commits no binaries, so anything needing a real image makes
    # one; doing it here keeps it out of the test's own runtime.
    set(ADE_TEST_IMAGE "${CMAKE_CURRENT_BINARY_DIR}/gui-fixture.adf")
    # A partitioned hard disk too, since a device is a different shape rather
    # than a bigger floppy: it holds no volume of its own (F-018).
    set(ADE_TEST_DEVICE "${CMAKE_CURRENT_BINARY_DIR}/gui-fixture.hdf")
    # A dataset naming the floppy fixture, so identification-on-open has
    # something to identify against (F-013). Generated with the image, because
    # the datfile matches it by CRC32 and the two must be made together.
    set(ADE_TEST_DATFILES "${CMAKE_CURRENT_BINARY_DIR}/datfiles")

    # Always re-run, like the bridge above, rather than treating the images as
    # build outputs that exist or do not. As outputs they went stale the moment
    # the generator changed: a test asserting on a file the fixture tool had
    # been taught to write would fail against the version written before it,
    # and the failure looks like a bug in the window. cargo is incremental and
    # knows whether anything changed; CMake, given only the image path, does
    # not.
    add_custom_target(ade_gui_fixture ALL
        COMMAND ${CMAKE_COMMAND} -E make_directory "${ADE_TEST_DATFILES}"
        COMMAND cargo run --quiet ${CARGO_FLAG} -p ade-fixtures --bin mkfixture
                --manifest-path "${ADE_ROOT}/Cargo.toml" -- "${ADE_TEST_IMAGE}"
                --datfile "${ADE_TEST_DATFILES}/fixture.dat"
        COMMAND cargo run --quiet ${CARGO_FLAG} -p ade-fixtures --bin mkfixture
                --manifest-path "${ADE_ROOT}/Cargo.toml" -- "${ADE_TEST_DEVICE}" --device
        BYPRODUCTS "${ADE_TEST_IMAGE}" "${ADE_TEST_DEVICE}"
        COMMENT "Generating the GUI test images"
        VERBATIM
    )
    add_dependencies(ade-gui-tests ade_gui_fixture)
    target_compile_definitions(ade-gui-tests PRIVATE ADE_TEST_IMAGE="${ADE_TEST_IMAGE}"
                                                     ADE_TEST_DEVICE="${ADE_TEST_DEVICE}"
                                                     ADE_TEST_DATFILES="${ADE_TEST_DATFILES}")
    target_link_libraries(ade-gui-tests PRIVATE Qt6::Widgets Qt6::Test "${ADE_LIB}" pthread dl m)
    target_compile_options(ade-gui-tests PRIVATE -Wall -Wextra)
    add_test(NAME gui COMMAND ade-gui-tests)
    # No X server in CI, and none needed.
    set_tests_properties(gui PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
endif()
