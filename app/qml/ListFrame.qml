import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A rounded, raised box with a scrolling list inside (the Section look, for
// lists too long to lay out in full). Use ListRow as the delegate.
Rectangle {
    id: frame

    property alias model: list.model
    property alias delegate: list.delegate
    property alias currentIndex: list.currentIndex
    property alias list: list
    property string emptyText

    radius: 10
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
        clip: true
        keyNavigationEnabled: true
        boundsBehavior: Flickable.StopAtBounds
        highlightMoveDuration: 0
        activeFocusOnTab: true
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
            policy: QQC2.ScrollBar.AsNeeded
        }
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
