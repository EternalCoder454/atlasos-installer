import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A scrolling page with a large bold title and generous, centred margins.
Item {
    id: root

    property string title
    default property alias content: col.data
    readonly property real maxContentWidth: Kirigami.Units.gridUnit * 38

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: width
        // The scrollbar overlays the content. The desktop style would reserve
        // its width as padding, which narrows the viewport whenever it shows
        // and makes a binding loop on implicitWidth.
        leftPadding: 0
        rightPadding: 0
        topPadding: 0
        bottomPadding: 0
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        // A slim overlay scrollbar instead of the classic one with arrows.
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
            parent: scroll
            x: scroll.width - width
            height: scroll.height
            policy: QQC2.ScrollBar.AsNeeded
            implicitWidth: 10
            padding: 2
            contentItem: Rectangle {
                implicitWidth: 6
                radius: width / 2
                color: Qt.alpha(Kirigami.Theme.textColor, parent.pressed ? 0.45 : parent.hovered ? 0.35 : 0.22)
                opacity: parent.active ? 1 : 0
                Behavior on opacity {
                    NumberAnimation {
                        duration: Kirigami.Units.longDuration
                    }
                }
            }
            background: null
        }

        Item {
            width: scroll.width
            implicitHeight: col.implicitHeight + Kirigami.Units.gridUnit * 3

            ColumnLayout {
                id: col
                y: Kirigami.Units.gridUnit * 1.5
                width: Math.min(parent.width - Kirigami.Units.gridUnit * 3, root.maxContentWidth)
                x: Math.round((parent.width - width) / 2)
                spacing: Kirigami.Units.gridUnit * 1.2

                QQC2.Label {
                    Layout.fillWidth: true
                    text: root.title
                    font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
                    font.bold: true
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    Accessible.role: Accessible.Heading
                }
            }
        }
    }

    // A plain ScrollView doesn't follow keyboard focus: tabbing to a control
    // below the fold would leave it off screen. Scroll just enough to show it.
    // Not for a click: scrolling under the pointer could drop the click.
    function ensureVisible(item) {
        const flick = scroll.contentItem as Flickable;
        if (!item || !flick || item.focusReason === Qt.MouseFocusReason)
            return;
        for (let p = item.parent; p !== col; p = p.parent) {
            if (!p)
                return;
        }
        const r = item.mapToItem(flick.contentItem, 0, 0, item.width, item.height);
        const top = flick.contentY;
        const bottom = top + flick.height;
        if (r.y >= top && r.y + r.height <= bottom)
            return;
        const margin = Kirigami.Units.largeSpacing;
        const maxY = Math.max(0, flick.contentHeight - flick.height);
        flick.cancelFlick();
        if (r.y < top)
            flick.contentY = Math.max(0, r.y - margin);
        else
            // Keep the top in view when the item is taller than the page.
            flick.contentY = Math.min(maxY, r.y - margin, r.y + r.height + margin - flick.height);
    }

    Connections {
        target: root.Window.window
        enabled: root.visible
        function onActiveFocusItemChanged() {
            root.ensureVisible(root.Window.activeFocusItem);
        }
    }
}
