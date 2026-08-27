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
    src/MainWindow.h
    src/ImageTree.h
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
        src/MainWindow.h
        src/ImageTree.h
        src/Image.h
    )
    add_dependencies(ade-gui-tests ade_bridge)
    target_include_directories(ade-gui-tests PRIVATE "${ADE_ROOT}/bridge/include" src)
    # Generate the test image at build time with the engine's own fixture
    # tool. D-010 commits no binaries, so anything needing a real image makes
    # one; doing it here keeps it out of the test's own runtime.
    set(ADE_TEST_IMAGE "${CMAKE_CURRENT_BINARY_DIR}/gui-fixture.adf")
    add_custom_command(
        OUTPUT "${ADE_TEST_IMAGE}"
        COMMAND cargo run --quiet ${CARGO_FLAG} -p ade-fixtures --bin mkfixture
                --manifest-path "${ADE_ROOT}/Cargo.toml" -- "${ADE_TEST_IMAGE}"
        COMMENT "Generating the GUI test image"
        VERBATIM
    )
    add_custom_target(ade_gui_fixture DEPENDS "${ADE_TEST_IMAGE}")
    add_dependencies(ade-gui-tests ade_gui_fixture)
    target_compile_definitions(ade-gui-tests PRIVATE ADE_TEST_IMAGE="${ADE_TEST_IMAGE}")
    target_link_libraries(ade-gui-tests PRIVATE Qt6::Widgets Qt6::Test "${ADE_LIB}" pthread dl m)
    target_compile_options(ade-gui-tests PRIVATE -Wall -Wextra)
    add_test(NAME gui COMMAND ade-gui-tests)
    # No X server in CI, and none needed.
    set_tests_properties(gui PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
endif()
