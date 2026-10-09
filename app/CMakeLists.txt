cmake_minimum_required(VERSION 3.24)
project(telamon-installer VERSION 0.3.0 LANGUAGES CXX)

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
    CRATES telamon-installer LOCKED)
corrosion_set_env_vars(telamon_installer QMAKE=${QMAKE6})

qt_add_executable(telamon-installer-app cpp/main.cpp)
set_target_properties(telamon-installer-app PROPERTIES OUTPUT_NAME telamon-installer)
# The hardening Fedora's package flags give (docs/SECURITY.md, "Build
# hardening"; scripts/check-hardening.sh reads it back): a position-independent
# program, stack protectors, full RELRO, no executable stack.
target_compile_options(telamon-installer-app PRIVATE -fPIE -fstack-protector-strong
    -fstack-clash-protection -fcf-protection=full)
target_link_options(telamon-installer-app PRIVATE -pie -Wl,-z,relro,-z,now,-z,noexecstack)
target_compile_definitions(telamon-installer-app PRIVATE TELAMON_INSTALLER_VERSION="${PROJECT_VERSION}")
# The QML type registration includes localeinfo.h by its bare name.
target_include_directories(telamon-installer-app PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/cpp)

# Telamon.Ui, the shared Telamon look, is installed like Kirigami (telamon-ui, from
# atlas-framework): QML imports it and nothing links it. qmlcachegen and
# qmllint read its types from the installed copy. The live ISO runs on the
# Telamon OS image, which has it; app/dev.sh puts it in the build container.
set(TELAMON_UI_QMLDIR "${QT6_INSTALL_PREFIX}/${QT6_INSTALL_QML}/Telamon/Ui/qmldir")
if(NOT EXISTS "${TELAMON_UI_QMLDIR}")
    message(FATAL_ERROR "Telamon.Ui is not installed: build in app/dev.sh's container")
endif()
# Telamon.Ui 2.0.0 or newer: its form controls and spinner. An older one would
# build and then fail to load the pages, so stop here. (For an ISO: the
# Telamon OS image it embeds must ship telamon-ui >= 2.0.0.)
file(READ "${TELAMON_UI_QMLDIR}" TELAMON_UI_TYPES)
foreach(type TelamonTextField TelamonComboBox TelamonSpinner)
    if(NOT TELAMON_UI_TYPES MATCHES "(^|\n)${type} ")
        message(FATAL_ERROR "The installed Telamon.Ui has no ${type}: the installer needs telamon-ui 2.0.0 or newer")
    endif()
endforeach()

# qmlcachegen compiles every QML file to C++ at build time; the app reads no
# .qml from disk at run time.
qt_add_qml_module(telamon-installer-app
    URI net.eterneon.telamon.installer
    VERSION 1.0
    SOURCES cpp/localeinfo.h cpp/localeinfo.cpp
    QML_FILES
        qml/Main.qml qml/InstallerPage.qml qml/ChoiceCard.qml
        qml/ListFrame.qml qml/ListRow.qml qml/BrandPanel.qml qml/BackButton.qml
        qml/WelcomePage.qml qml/KeyboardPage.qml qml/WifiPage.qml qml/AppsPage.qml qml/DiskPage.qml
        qml/ReviewPage.qml qml/ProgressPage.qml qml/RestartPage.qml
    RESOURCES
        data/telamon-logo.svg
        data/art/welcome.svg data/art/keyboard.svg data/art/wifi.svg data/art/apps.svg data/art/disk.svg
        data/art/review.svg data/art/progress.svg data/art/restart.svg
)

target_link_libraries(telamon-installer-app PRIVATE
    telamon_installer
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets)

install(TARGETS telamon-installer-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.telamon.installer.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
install(FILES data/telamon-logo.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps
        RENAME net.eterneon.telamon.installer.svg)
