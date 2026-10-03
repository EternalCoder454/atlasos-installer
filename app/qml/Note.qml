import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A rounded note in the style of the grouped sections: an icon and a
// paragraph, tinted by kind ("info", "warning" or "error").
Rectangle {
    id: note

    property string kind: "info"
    property string text

    readonly property color tint: note.kind === "error" ? Kirigami.Theme.negativeTextColor : note.kind === "warning" ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.highlightColor

    Layout.fillWidth: true
    implicitHeight: row.implicitHeight + Kirigami.Units.largeSpacing * 2
    radius: 10
    color: Qt.alpha(note.tint, 0.1)
    border.width: 1
    border.color: Qt.alpha(note.tint, 0.3)
    Accessible.role: Accessible.StaticText
    Accessible.name: note.text

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing
        anchors.leftMargin: Kirigami.Units.largeSpacing * 1.5
        anchors.rightMargin: Kirigami.Units.largeSpacing * 1.5
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
            source: note.kind === "error" ? "dialog-error" : note.kind === "warning" ? "dialog-warning" : "dialog-information"
            Accessible.ignored: true
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            text: note.text
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
        }
    }
}
