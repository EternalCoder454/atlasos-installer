pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Telamon.Ui

// Step 4: which apps to add at the first start. Built from the catalog
// (firstboot/apps.json): a new entry there shows up here with no change.
InstallerPage {
    id: page

    required property var app

    readonly property var catalog: JSON.parse(page.app.backend.appsJson)
    readonly property var browsers: page.catalog.filter(a => a.group === "browser")
    readonly property var tools: page.catalog.filter(a => a.group === "developer")
    readonly property var aiTools: page.catalog.filter(a => a.group === "ai")
    readonly property bool aiChosen: page.aiTools.some(a => page.app.apps.includes(a.id))
    readonly property var chosenBrowser: page.browsers.find(a => page.app.apps.includes(a.id)) || null

    title: qsTr("Pick Your Apps")
    subtitle: qsTr("They're added the first time Telamon OS starts, once it's online. You can change these any time.")
    onPrimary: page.app.next()
    onBack: page.app.previous()

    // At most one browser: choosing one replaces the other.
    function setBrowser(id) {
        const others = page.app.apps.filter(i => !page.browsers.some(b => b.id === i));
        page.app.apps = id.length > 0 ? others.concat([id]) : others;
    }

    function setTool(id, on) {
        const others = page.app.apps.filter(i => i !== id);
        page.app.apps = on ? others.concat([id]) : others;
    }

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff
        QQC2.ScrollBar.vertical: TelamonScrollBar {}

        ColumnLayout {
            width: page.contentWidth
            x: Math.round((scroll.width - width) / 2)
            spacing: Kirigami.Units.gridUnit

            Section {
                title: qsTr("Web Browser")
                visible: page.browsers.length > 0
                Repeater {
                    model: page.browsers
                    SectionRow {
                        required property var modelData
                        title: modelData.name
                        subtitle: modelData.summary
                        radio: true
                        clickable: true
                        checkmark: page.chosenBrowser !== null && page.chosenBrowser.id === modelData.id
                        onClicked: page.setBrowser(modelData.id)
                    }
                }
                SectionRow {
                    title: qsTr("None")
                    radio: true
                    clickable: true
                    checkmark: page.chosenBrowser === null
                    onClicked: page.setBrowser("")
                }
            }

            Section {
                title: qsTr("Developer Tools")
                visible: page.tools.length > 0
                Repeater {
                    model: page.tools
                    SectionRow {
                        required property var modelData
                        title: modelData.name
                        subtitle: modelData.summary
                        showSwitch: true
                        switchChecked: page.app.apps.includes(modelData.id)
                        onSwitchToggled: checked => page.setTool(modelData.id, checked)
                    }
                }
            }

            // The warning appears under the switches once one is on, where
            // the user is looking, instead of above them while both are off.
            ColumnLayout {
                Layout.fillWidth: true
                visible: page.aiTools.length > 0
                spacing: Kirigami.Units.largeSpacing

                Section {
                    title: qsTr("Local AI")
                    footer: qsTr("No AI models are downloaded now: they are several gigabytes each. You choose and download them once Telamon OS is running.")
                    Repeater {
                        model: page.aiTools
                        SectionRow {
                            required property var modelData
                            title: modelData.name
                            subtitle: modelData.summary
                            showSwitch: true
                            switchChecked: page.app.apps.includes(modelData.id)
                            onSwitchToggled: checked => page.setTool(modelData.id, checked)
                        }
                    }
                }
                InfoBanner {
                    Layout.fillWidth: true
                    shown: page.aiChosen
                    type: "warning"
                    text: qsTr("Local AI usually needs a dedicated graphics card (GPU) with enough video memory. On integrated graphics, or with only the processor, it runs slowly.")
                }
            }

            Item {
                implicitHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
