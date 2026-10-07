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
    readonly property var chosenBrowser: page.browsers.find(a => page.app.apps.includes(a.id)) || null

    title: qsTr("Pick your apps")
    subtitle: qsTr("You can change these any time.")
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

        ColumnLayout {
            width: page.contentWidth
            x: Math.round((scroll.width - width) / 2)
            spacing: Kirigami.Units.gridUnit

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                text: qsTr("They're added the first time AtlasOS starts, once it's online.")
                wrapMode: Text.Wrap
                opacity: 0.65
            }

            Section {
                title: qsTr("Web browser")
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
                title: qsTr("Developer tools")
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

            // The warning sits between the heading and the rows, so the
            // heading is drawn here and not by the Section.
            ColumnLayout {
                Layout.fillWidth: true
                visible: page.aiTools.length > 0
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    text: qsTr("Local AI")
                    font.bold: true
                    opacity: 0.65
                    Accessible.role: Accessible.Heading
                }
                InfoBanner {
                    Layout.fillWidth: true
                    type: "warning"
                    text: qsTr("Local AI usually needs a dedicated graphics card (GPU) with enough video memory. On integrated graphics, or with only the processor, it runs slowly.")
                }
                Section {
                    footer: qsTr("Neither one downloads any AI models, which are several gigabytes each. You choose and download them later, once Telamon OS is running.")
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
            }

            Item {
                implicitHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
