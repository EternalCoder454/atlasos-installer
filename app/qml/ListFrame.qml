import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// A rounded, raised box with a scrolling list inside (the Section look, for
// lists too long to lay out in full). Use ListRow as the delegate.
Rectangle {
    id: frame

    property alias model: list.model
    property alias delegate: list.delegate
    property alias currentIndex: list.currentIndex
    property alias list: list
    property string emptyText
    // A short list gets a short frame, not an empty box down to the footer.
    property bool fitContent: false

    Layout.maximumHeight: frame.fitContent ? list.contentHeight + 2 : Number.POSITIVE_INFINITY

    radius: TelamonStyle.radiusLarge
    // The same card as a Section, so lists and sections sit alike.
    color: TelamonStyle.surface
    border.width: 1
    border.color: TelamonStyle.separator
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
        anchors.rightMargin: 1 + (bar.size < 1 ? bar.width : 0)
        clip: true
        keyNavigationEnabled: true
        boundsBehavior: Flickable.StopAtBounds
        highlightMoveDuration: 0
        activeFocusOnTab: true
        QQC2.ScrollBar.vertical: bar
    }
    TelamonScrollBar {
        id: bar
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        anchors.margins: 1
    }

    QQC2.Label {
        textFormat: Text.PlainText
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 2
        visible: list.count === 0 && frame.emptyText.length > 0
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: frame.emptyText
        opacity: 0.6
    }
}
