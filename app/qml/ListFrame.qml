import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// A rounded, raised box with a scrolling list inside (the Section look, for
// lists too long to lay out in full). Use ListRow as the delegate.
Rectangle {
    id: frame

    property alias model: list.model
    property alias delegate: list.delegate
    property alias currentIndex: list.currentIndex
    property alias list: list
    property string emptyText

    radius: AtlasStyle.radiusLarge
    color: Kirigami.Theme.backgroundColor.hslLightness > 0.5 ? Qt.lighter(Kirigami.Theme.backgroundColor, 1.5) : Qt.tint(Kirigami.Theme.backgroundColor, Qt.rgba(1, 1, 1, 0.06))
    border.width: 1
    border.color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
    clip: true

    function positionAt(index) {
        list.positionViewAtIndex(index, ListView.Center);
    }

    ListView {
        id: list
        anchors.fill: parent
        anchors.margins: 1
        // Beside the scroll bar, not under it: rows end where it starts, so
        // nothing at their right edge (a lock, a checkmark) is covered.
        anchors.rightMargin: 1 + (bar.visible ? bar.width : 0)
        clip: true
        keyNavigationEnabled: true
        boundsBehavior: Flickable.StopAtBounds
        highlightMoveDuration: 0
        activeFocusOnTab: true
        QQC2.ScrollBar.vertical: bar
    }
    QQC2.ScrollBar {
        id: bar
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        anchors.margins: 1
        policy: QQC2.ScrollBar.AsNeeded
    }

    QQC2.Label {
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 2
        visible: list.count === 0 && frame.emptyText.length > 0
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: frame.emptyText
        opacity: 0.6
    }
}
