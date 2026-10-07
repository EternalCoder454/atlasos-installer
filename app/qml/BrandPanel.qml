pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// The violet panel beside every page: the Telamon OS mark, a drawing for the
// current step, and the steps as quiet dots. Done steps can be clicked to
// go back. Its colours are the brand's, not the theme's, so it looks the
// same with any accent colour.
Rectangle {
    id: panel

    // [{key, title}]
    required property var steps
    required property string current
    // Index into steps of the furthest step reached.
    required property int reached
    required property bool canGoBack
    property bool demo: false

    signal stepClicked(string key)

    readonly property int currentIndex: panel.steps.findIndex(s => s.key === panel.current)
    readonly property bool darkTheme: Kirigami.ColorUtils.brightnessForColor(Kirigami.Theme.backgroundColor) === Kirigami.ColorUtils.Dark
    readonly property color ink: "#ffffff"

    gradient: Gradient {
        GradientStop { position: 0; color: panel.darkTheme ? "#5545c9" : "#6a58e6" }
        GradientStop { position: 0.55; color: panel.darkTheme ? "#3b2fa0" : "#4b3cc0" }
        GradientStop { position: 1; color: panel.darkTheme ? "#211a66" : "#2f2690" }
    }
    clip: true

    // Two soft rings for depth.
    Rectangle {
        width: panel.width * 1.4
        height: width
        radius: width / 2
        // Plain x is not mirrored for right-to-left languages.
        x: LayoutMirroring.enabled ? panel.width * 0.75 - width : panel.width * 0.25
        y: -height * 0.55
        color: "transparent"
        border.width: Kirigami.Units.gridUnit * 3
        border.color: Qt.alpha(panel.ink, 0.05)
    }
    Rectangle {
        width: panel.width * 1.1
        height: width
        radius: width / 2
        x: LayoutMirroring.enabled ? panel.width - width * 0.5 : -width * 0.5
        y: panel.height - height * 0.45
        color: Qt.alpha(panel.ink, 0.04)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.gridUnit * 1.5
        spacing: Kirigami.Units.gridUnit

        RowLayout {
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Icon {
                implicitWidth: Kirigami.Units.iconSizes.medium
                implicitHeight: Kirigami.Units.iconSizes.medium
                source: "qrc:/qt/qml/net/eterneon/telamon/installer/data/atlasos-logo.svg"
                isMask: true
                color: panel.ink
                Accessible.ignored: true
            }
            Text {
                text: "Telamon OS"
                color: panel.ink
                font.family: Kirigami.Theme.defaultFont.family
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.3
                font.weight: Font.DemiBold
                textFormat: Text.PlainText
            }
        }

        // The drawing for the current step, cross-faded.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Repeater {
                model: panel.steps
                Image {
                    id: art
                    required property var modelData
                    anchors.centerIn: parent
                    readonly property real w: Math.min(parent.width, Kirigami.Units.gridUnit * 13)
                    width: w
                    height: w * 0.75
                    sourceSize.width: w * 2
                    sourceSize.height: w * 1.5
                    fillMode: Image.PreserveAspectFit
                    source: "qrc:/qt/qml/net/eterneon/telamon/installer/data/art/" + art.modelData.key + ".svg"
                    opacity: art.modelData.key === panel.current ? 1 : 0
                    visible: opacity > 0
                    scale: art.modelData.key === panel.current ? 1 : 0.96
                    Accessible.ignored: true
                    Behavior on opacity {
                        NumberAnimation { duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
                    }
                    Behavior on scale {
                        NumberAnimation { duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
                    }
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Steps")

            Repeater {
                model: panel.steps
                T.AbstractButton {
                    id: step
                    required property var modelData
                    required property int index
                    readonly property bool isCurrent: step.index === panel.currentIndex
                    readonly property bool done: step.index <= panel.reached && !step.isCurrent
                    readonly property bool clickable: step.done && panel.canGoBack && step.index < panel.currentIndex

                    Layout.fillWidth: true
                    implicitHeight: Math.round(Kirigami.Units.gridUnit * 1.6)
                    text: step.modelData.title
                    hoverEnabled: step.clickable
                    // Kept while focused: a clicked step becomes the current
                    // one, and Qt refuses to drop the focus policy of the
                    // focused item. It goes once the focus moves on.
                    focusPolicy: step.clickable || step.activeFocus ? Qt.StrongFocus : Qt.NoFocus
                    Accessible.role: step.clickable ? Accessible.Button : Accessible.ListItem
                    Accessible.name: step.text
                    Accessible.selected: step.isCurrent
                    Accessible.description: step.isCurrent ? qsTr("Current step") : step.done ? qsTr("Done") : qsTr("Not done yet")
                    onClicked: if (step.clickable) panel.stepClicked(step.modelData.key)
                    Keys.onReturnPressed: event => {
                        if (step.clickable && !event.isAutoRepeat) panel.stepClicked(step.modelData.key);
                    }
                    Keys.onEnterPressed: event => {
                        if (step.clickable && !event.isAutoRepeat) panel.stepClicked(step.modelData.key);
                    }

                    // Swallow presses on steps that can't be opened.
                    MouseArea {
                        anchors.fill: parent
                        enabled: !step.clickable
                        acceptedButtons: Qt.AllButtons
                    }

                    background: Rectangle {
                        radius: 6
                        anchors.fill: parent
                        anchors.leftMargin: -Kirigami.Units.smallSpacing
                        color: Qt.alpha(panel.ink, step.down ? 0.14 : step.hovered ? 0.08 : 0)
                        border.width: step.visualFocus ? 2 : 0
                        border.color: Qt.alpha(panel.ink, 0.8)
                    }

                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing
                        Rectangle {
                            readonly property real size: step.isCurrent ? 8 : 6
                            Layout.preferredWidth: 10
                            Layout.preferredHeight: 10
                            color: "transparent"
                            Rectangle {
                                anchors.centerIn: parent
                                width: parent.size
                                height: width
                                radius: width / 2
                                color: Qt.alpha(panel.ink, step.isCurrent ? 1 : step.done ? 0.8 : 0.35)
                            }
                            Rectangle {
                                anchors.centerIn: parent
                                visible: step.isCurrent
                                width: 16
                                height: 16
                                radius: 8
                                color: Qt.alpha(panel.ink, 0.22)
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            text: step.text
                            color: panel.ink
                            // 0.72 keeps steps to come above 5:1 where the list sits.
                            opacity: step.isCurrent ? 1 : step.done ? 0.85 : 0.72
                            font.family: Kirigami.Theme.defaultFont.family
                            font.pointSize: Kirigami.Theme.defaultFont.pointSize
                            font.weight: step.isCurrent ? Font.DemiBold : Font.Normal
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }
                    }
                }
            }
        }

        Text {
            Layout.fillWidth: true
            visible: panel.demo
            text: qsTr("Demo: nothing here is real, and nothing is installed.")
            wrapMode: Text.Wrap
            color: "#ffd98a"
            font.family: Kirigami.Theme.defaultFont.family
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.9
            textFormat: Text.PlainText
        }
    }
}
