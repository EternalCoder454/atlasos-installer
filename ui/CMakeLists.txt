# Atlas.Ui: the shared look of Atlas apps (pill buttons, grouped sections,
# status hero, sidebar items, setup steps, progress bar). A static QML module
# compiled ahead of time by qmlcachegen. Use it from an app with:
#   add_subdirectory(<path to ui> atlas-ui)
#   target_link_libraries(<app> PRIVATE atlas-ui atlas-uiplugin)
# and, in QML, `import Atlas.Ui`.
qt_add_qml_module(atlas-ui
    STATIC
    URI Atlas.Ui
    VERSION 1.0
    QML_FILES
        AtlasButton.qml
        PrimaryButton.qml
        SecondaryButton.qml
        TextButton.qml
        MenuButton.qml
        Section.qml
        SectionRow.qml
        StatusHero.qml
        SidebarItem.qml
        AtlasPage.qml
        AtlasSwitch.qml
        ConfirmDialog.qml
        NotesText.qml
        StepItem.qml
        AtlasProgressBar.qml
)
target_link_libraries(atlas-ui PRIVATE Qt6::Quick Qt6::QuickControls2 Qt6::Qml)
