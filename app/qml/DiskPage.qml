pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 5: which disk, then erase it or use its free space.
InstallerPage {
    id: page

    required property var app

    readonly property var backend: page.app.backend
    readonly property var disks: JSON.parse(page.backend.disksJson)
    readonly property string diskState: page.backend.disksState
    readonly property var disk: page.app.disk
    readonly property bool modeOk: page.disk !== null && (page.app.mode === "erase" && page.disk.eraseOk || page.app.mode === "free-space" && page.disk.freeOk)

    title: qsTr("Where should AtlasOS go?")
    subtitle: qsTr("Choose a disk, then how to install on it.")
    primaryEnabled: page.diskState === "ready" && page.modeOk && page.app.encryptionReady
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
                AtlasSpinner {
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
                tint: page.diskState === "error" ? Kirigami.Theme.negativeTextColor : AtlasStyle.accent
                headline: page.diskState === "error" ? qsTr("Couldn't read the disks") : qsTr("No disk found")
                subtitle: page.diskState === "error" ? page.backend.disksError : qsTr("AtlasOS needs a disk of at least %1. Connect one, then look again.").arg(page.backend.minDiskSize)
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
                    text: qsTr("How to install")
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
                InfoBanner {
                    Layout.fillWidth: true
                    shown: page.disk !== null && page.disk.note.length > 0
                    type: "warning"
                    text: page.disk ? page.disk.note : ""
                }
            }

            Section {
                id: encryptionSection
                visible: page.diskState === "ready" && page.disk !== null && page.app.mode !== ""
                title: qsTr("Encryption")
                footer: page.app.encryption === "none" ? "" : qsTr("At start-up and when AtlasOS asks for the recovery key, you type with the keyboard layout you chose earlier.")

                SectionRow {
                    iconName: "lock"
                    title: qsTr("Encrypt this disk")
                    subtitle: page.backend.tpm2 ? qsTr("AtlasOS unlocks the disk by itself when this PC starts. Your files stay unreadable if the disk is taken out of this PC, or if the PC is sold or recycled. You'll get a recovery key at the end.") : qsTr("This PC has no security chip (TPM 2.0), so you'd type a password every time it starts.")
                    showSwitch: true
                    switchChecked: page.app.encrypt
                    onSwitchToggled: checked => page.app.encryptChoice = checked ? "on" : "off"
                }

                SectionRow {
                    visible: page.backend.tpm2 && page.app.encrypt
                    iconName: "input-dialpad-symbolic"
                    title: qsTr("Ask for a PIN when this PC starts")
                    subtitle: qsTr("Protects your files if the whole PC is stolen: nobody can start AtlasOS without the PIN.")
                    showSwitch: true
                    switchChecked: page.app.encryptPin
                    onSwitchToggled: checked => page.app.pinChoice = checked ? "on" : "off"
                }

                ColumnLayout {
                    id: secretBox
                    visible: page.app.needsSecret
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.largeSpacing
                    spacing: Kirigami.Units.largeSpacing

                    readonly property bool pin: page.app.encryption === "tpm-pin"

                    AtlasPasswordField {
                        id: secretField
                        Layout.fillWidth: true
                        placeholderText: secretBox.pin ? qsTr("PIN") : qsTr("Password")
                        Accessible.name: placeholderText
                        text: page.app.password
                        onTextChanged: page.app.password = text
                        onAccepted: confirmField.forceActiveFocus()
                    }
                    AtlasPasswordField {
                        id: confirmField
                        Layout.fillWidth: true
                        placeholderText: secretBox.pin ? qsTr("Confirm PIN") : qsTr("Confirm password")
                        Accessible.name: placeholderText
                        text: page.app.passwordConfirm
                        onTextChanged: page.app.passwordConfirm = text
                    }
                    QQC2.Label {
                        readonly property string problem: page.backend.secretHint(page.app.encryption, page.app.password, page.app.passwordConfirm)
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        font: Kirigami.Theme.smallFont
                        text: problem.length > 0 ? problem : secretBox.pin ? qsTr("Use at least 4 characters: letters without accents, numbers, spaces and symbols. At start-up, type it where it asks for the \"LUKS2 token PIN\". If you forget it, the recovery key opens the disk.") : qsTr("Use at least 8 characters: letters without accents, numbers, spaces and symbols. Write it down somewhere safe: if you forget it, only the recovery key opens the disk.")
                        color: problem.length > 0 ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                        opacity: problem.length > 0 ? 1 : 0.65
                    }
                }
            }

            InfoBanner {
                Layout.fillWidth: true
                shown: encryptionSection.visible && page.backend.tpm2 && page.app.encrypt && page.backend.secureBootOff
                type: "warning"
                text: qsTr("Secure Boot is off on this PC. Turn it on in the firmware settings for full protection.")
            }

            Item {
                implicitHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
