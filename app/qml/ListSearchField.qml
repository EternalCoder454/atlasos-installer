import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// A rounded search box. Down moves into the list below it. Not named
// SearchField: Atlas.Ui has one, and its import would hide this file.
AtlasTextField {
    id: field

    signal down

    Layout.fillWidth: true
    placeholderText: qsTr("Search")
    clearable: true
    leftPadding: Kirigami.Units.largeSpacing + Kirigami.Units.iconSizes.small + Kirigami.Units.smallSpacing * 2
    inputMethodHints: Qt.ImhNoPredictiveText
    Accessible.name: placeholderText
    Keys.onDownPressed: field.down()

    Kirigami.Icon {
        anchors.left: parent.left
        anchors.leftMargin: Kirigami.Units.largeSpacing
        // the field's pill, not the whole item (which has room for errorText)
        y: Math.round((field.background.height - height) / 2)
        width: Kirigami.Units.iconSizes.small
        height: width
        source: "search"
        isMask: true
        color: Kirigami.Theme.textColor
        opacity: 0.6
    }
}
