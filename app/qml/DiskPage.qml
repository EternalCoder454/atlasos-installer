pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 4: which disk, then erase it or use its free space.
InstallerPage {
    id: page

    required property var app

    readonly property var backend: page.app.backend
    readonly property var disks: JSON.parse(page.backend.disksJson)
    readonly property string diskState: page.backend.disksState
    readonly property var disk: page.app.disk
    readonly property bool modeOk: page.disk !== null && (page.app.mode === "erase" && page.disk.eraseOk || page.app.mode === "free-space" && page.disk.freeOk)

    title: qsTr("Choose a Disk")
    subtitle: qsTr("Where should AtlasOS go?")
    primaryEnabled: page.diskState === "ready" && page.modeOk
    onPrimary: page.app.next()
    onBack: page.app.previous()

    // A choice held for another disk doesn't carry over. Prefer installing
    // alongside, which loses nothing; erase is preselected only for an empty
    // disk, and is otherwise always the user's own click.
    function choose(d) {
        if (page.app.disk === null || page.app.disk.id !== d.id) {
            page.app.mode = "";
        }
        page.app.disk = d;
        if (!(page.app.mode === "erase" && d.eraseOk || page.app.mode === "free-space" && d.freeOk)) {
            page.app.mode = d.freeOk ? "free-space" : d.eraseOk && d.empty ? "erase" : "";
        }
    }

    // The chosen disk went away or changed: don't swap in another unasked.
    property bool choiceLost: false

    // One disk to choose from: choose it.
    function chooseOnlyDisk() {
        const usable = page.disks.filter(d => d.selectable);
        if (page.app.disk === null && !page.choiceLost && page.diskState === "ready" && usable.length === 1) {
            page.choose(usable[0]);
        }
    }

    // The list is set before the state turns "ready".
    onDiskStateChanged: page.chooseOnlyDisk()

    // The list may be old by now (a disk plugged in or changed): read it
    // again on every visit. onDisksChanged then checks the held choice.
    Component.onCompleted: {
        if (page.diskState !== "loading") {
            page.backend.refreshDisks();
        }
    }

    // After a fresh list, the chosen disk must still be in it, unchanged.
    onDisksChanged: {
        if (page.app.disk === null) {
            page.chooseOnlyDisk();
            return;
        }
        const same = page.disks.find(d => d.id === page.app.disk.id && d.fingerprint === page.app.disk.fingerprint && d.selectable);
        if (same) {
            page.app.disk = same;
        } else {
            page.choiceLost = true;
            page.app.disk = null;
            page.app.mode = "";
        }
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

            // Looking for disks
            RowLayout {
                visible: page.diskState === "loading"
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: Kirigami.Units.gridUnit * 3
                spacing: Kirigami.Units.largeSpacing
                QQC2.BusyIndicator {
                    running: parent.visible
                }
                QQC2.Label {
                    text: qsTr("Looking for disks…")
                    opacity: 0.7
                }
            }

            // Couldn't list them, or nothing to offer
            StatusHero {
                visible: page.diskState === "error" || page.diskState === "ready" && page.disks.length === 0
                Layout.topMargin: Kirigami.Units.gridUnit * 2
                iconName: page.diskState === "error" ? "window-close-symbolic" : "drive-harddisk-symbolic"
                tint: page.diskState === "error" ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.highlightColor
                headline: page.diskState === "error" ? qsTr("Couldn't Read the Disks") : qsTr("No Disk Found")
                subtitle: page.diskState === "error" ? page.backend.disksError : qsTr("AtlasOS needs a disk of at least 40 GB. Connect one, then look again.")
                SecondaryButton {
                    text: qsTr("Look Again")
                    onClicked: page.backend.refreshDisks()
                }
            }

            Section {
                visible: page.diskState === "ready" && page.disks.length > 0
                title: qsTr("Disks")
                Repeater {
                    model: page.disks
                    SectionRow {
                        id: diskRow
                        required property var modelData
                        title: modelData.title
                        subtitle: modelData.selectable ? modelData.subtitle : modelData.subtitle + "\n" + modelData.reason
                        iconName: modelData.icon
                        radio: true
                        clickable: modelData.selectable
                        checkmark: page.disk !== null && page.disk.id === modelData.id
                        opacity: modelData.selectable ? 1 : 0.5
                        onClicked: page.choose(modelData)

                        QQC2.Label {
                            visible: diskRow.modelData.usb
                            text: qsTr("USB")
                            font: Kirigami.Theme.smallFont
                            leftPadding: Kirigami.Units.smallSpacing * 1.5
                            rightPadding: leftPadding
                            topPadding: 1
                            bottomPadding: 1
                            background: Rectangle {
                                radius: height / 2
                                color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
                            }
                        }
                    }
                }
            }

            TextButton {
                visible: page.diskState === "ready" && page.disks.length > 0
                Layout.alignment: Qt.AlignRight
                Layout.topMargin: -Kirigami.Units.largeSpacing
                text: qsTr("Look for Disks Again")
                onClicked: page.backend.refreshDisks()
            }

            ColumnLayout {
                visible: page.diskState === "ready" && page.disk !== null
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing

                QQC2.Label {
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    text: qsTr("How to Install")
                    font.bold: true
                    opacity: 0.65
                    Accessible.role: Accessible.Heading
                }
                ChoiceCard {
                    title: page.disk ? page.disk.freeTitle : ""
                    text: page.disk ? page.disk.freeText : ""
                    iconName: "view-split-left-right"
                    possible: page.disk !== null && page.disk.freeOk
                    selected: page.app.mode === "free-space"
                    onClicked: page.app.mode = "free-space"
                }
                ChoiceCard {
                    title: page.disk ? page.disk.eraseTitle : ""
                    text: page.disk ? page.disk.eraseText : ""
                    iconName: "edit-clear-all"
                    destructive: true
                    possible: page.disk !== null && page.disk.eraseOk
                    selected: page.app.mode === "erase"
                    onClicked: page.app.mode = "erase"
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    visible: page.app.mode === ""
                    text: qsTr("Choose how to install AtlasOS on this disk.")
                    wrapMode: Text.Wrap
                    opacity: 0.65
                }
                Note {
                    Layout.fillWidth: true
                    visible: page.disk !== null && page.disk.note.length > 0
                    kind: "warning"
                    text: page.disk ? page.disk.note : ""
                }
            }

            Item {
                implicitHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
