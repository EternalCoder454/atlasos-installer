import QtQuick
import QtQuick.Layouts
import QtQuick.Shapes
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// Big centred status: a round icon badge (with a ring for busy/progress), a
// headline, a subtitle, and the action buttons you put inside.
ColumnLayout {
    id: root

    property string iconName
    property string headline
    property string subtitle
    // Spinning ring while true.
    property bool busy: false
    // 0..1 draws a progress ring; negative means none.
    property real progress: -1
    property color tint: Kirigami.Theme.highlightColor
    default property alias actions: actionRow.data

    Layout.fillWidth: true
    spacing: Kirigami.Units.largeSpacing

    // The headline is the page's state. Say it when it changes, so a screen
    // reader hears the new state without a banner repeating it.
    onHeadlineChanged: {
        if (root.visible && root.headline.length > 0) {
            Accessible.announce(root.headline);
        }
    }

    Item {
        id: badge
        readonly property real size: Math.round(Kirigami.Units.gridUnit * 5)
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: size
        Layout.preferredHeight: size

        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: Qt.alpha(root.tint, 0.14)
        }
        Kirigami.Icon {
            anchors.centerIn: parent
            width: Math.round(badge.size * 0.5)
            height: width
            source: root.iconName
            isMask: true
            color: root.tint
        }
        Shape {
            id: ring
            anchors.fill: parent
            visible: root.busy || root.progress >= 0
            preferredRendererType: Shape.CurveRenderer
            rotation: 0
            ShapePath {
                strokeColor: root.tint
                strokeWidth: 4
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                PathAngleArc {
                    centerX: badge.size / 2
                    centerY: badge.size / 2
                    radiusX: badge.size / 2 - 2
                    radiusY: radiusX
                    startAngle: -90
                    sweepAngle: root.progress >= 0 ? 360 * Math.max(0.02, root.progress) : 100
                }
            }
            RotationAnimator on rotation {
                running: root.busy && root.progress < 0 && Kirigami.Units.longDuration > 0
                from: 0
                to: 360
                loops: Animation.Infinite
                duration: Kirigami.Units.veryLongDuration * 3
            }
        }
    }

    Kirigami.Heading {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        level: 1
        font.weight: Font.DemiBold
        wrapMode: Text.Wrap
        text: root.headline
        textFormat: Text.PlainText
    }
    QQC2.Label {
        Layout.fillWidth: true
        visible: root.subtitle.length > 0
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        opacity: 0.7
        text: root.subtitle
        textFormat: Text.PlainText
    }
    // Actions sit side by side when they fit, and stack when they do not.
    GridLayout {
        id: actionRow
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: Kirigami.Units.smallSpacing
        columnSpacing: Kirigami.Units.largeSpacing
        rowSpacing: Kirigami.Units.smallSpacing
        columns: root.sideBySide ? 100 : 1
    }
    readonly property real wideWidth: {
        var w = 0, n = 0;
        for (var i = 0; i < actionRow.children.length; ++i) {
            var c = actionRow.children[i];
            if (c.visible) {
                w += c.implicitWidth;
                n++;
            }
        }
        return w + Math.max(0, n - 1) * Kirigami.Units.largeSpacing;
    }
    readonly property bool sideBySide: wideWidth <= root.width
}
