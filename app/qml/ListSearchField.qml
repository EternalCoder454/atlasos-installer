import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A rounded search box. Down moves into the list below it. Not named
// SearchField: Atlas.Ui has one, and its import would hide this file.
QQC2.TextField {
    id: field

    signal down

    Layout.fillWidth: true
    placeholderText: qsTr("Search")
    leftPadding: Kirigami.Units.largeSpacing + Kirigami.Units.iconSizes.small + Kirigami.Units.smallSpacing
    inputMethodHints: Qt.ImhNoPredictiveText
    Accessible.name: placeholderText
    Keys.onDownPressed: field.down()

    Kirigami.Icon {
        anchors.left: parent.left
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.verticalCenter: parent.verticalCenter
        width: Kirigami.Units.iconSizes.small
        height: width
        source: "search"
        isMask: true
        color: Kirigami.Theme.textColor
        opacity: 0.6
    }
}
