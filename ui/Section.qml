import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A rounded card that groups SectionRows (or any items) on a raised background.
ColumnLayout {
    id: root

    property string title
    property string footer
    default property alias content: col.data

    Layout.fillWidth: true
    spacing: Kirigami.Units.smallSpacing

    QQC2.Label {
        visible: root.title.length > 0
        Layout.leftMargin: Kirigami.Units.largeSpacing
        text: root.title
        font.bold: true
        opacity: 0.65
        textFormat: Text.PlainText
        Accessible.role: Accessible.Heading
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: col.implicitHeight + 2
        radius: 10
        // Slightly raised over the page in both light and dark.
        color: Kirigami.Theme.backgroundColor.hslLightness > 0.5 ? Qt.lighter(Kirigami.Theme.backgroundColor, 1.5) : Qt.tint(Kirigami.Theme.backgroundColor, Qt.rgba(1, 1, 1, 0.06))
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)

        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: 1
            spacing: 0
        }
    }

    Text {
        visible: root.footer.length > 0
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        text: root.footer
        wrapMode: Text.Wrap
        font.family: Kirigami.Theme.defaultFont.family
        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.92
        color: Qt.alpha(Kirigami.Theme.textColor, 0.65)
        textFormat: Text.PlainText
    }
}
