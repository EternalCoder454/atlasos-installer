cmake_minimum_required(VERSION 3.24)
project(atlas-installer VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX /usr CACHE PATH "" FORCE)
endif()
include(GNUInstallDirs)
find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Widgets)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)
find_package(Corrosion REQUIRED)

# Rust library (the CXX-Qt QObject). cxx-qt-build finds Qt via $QMAKE.
find_program(QMAKE6 NAMES qmake6 qmake REQUIRED)
corrosion_import_crate(MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
    CRATES atlas-installer LOCKED)
corrosion_set_env_vars(atlas_installer QMAKE=${QMAKE6})

qt_add_executable(atlas-installer-app cpp/main.cpp)
set_target_properties(atlas-installer-app PROPERTIES OUTPUT_NAME atlas-installer)
target_compile_definitions(atlas-installer-app PRIVATE ATLAS_INSTALLER_VERSION="${PROJECT_VERSION}")
# The QML type registration includes localeinfo.h by its bare name.
target_include_directories(atlas-installer-app PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/cpp)

# qmlcachegen compiles every QML file to C++ at build time; the app reads no
# .qml from disk at run time.
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../ui atlas-ui)
qt_add_qml_module(atlas-installer-app
    URI net.eterneon.atlas.installer
    VERSION 1.0
    SOURCES cpp/localeinfo.h cpp/localeinfo.cpp
    QML_FILES
        qml/Main.qml qml/InstallerPage.qml qml/SearchField.qml qml/ChoiceCard.qml qml/Note.qml
        qml/ListFrame.qml qml/ListRow.qml
        qml/WelcomePage.qml qml/KeyboardPage.qml qml/WifiPage.qml qml/DiskPage.qml
        qml/ReviewPage.qml qml/ProgressPage.qml qml/RestartPage.qml
    RESOURCES
        data/atlasos-logo.svg data/check.svg
)

target_link_libraries(atlas-installer-app PRIVATE
    atlas_installer atlas-ui atlas-uiplugin
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets)

install(TARGETS atlas-installer-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.atlas.installer.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
install(FILES data/atlasos-logo.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps
        RENAME net.eterneon.atlas.installer.svg)
