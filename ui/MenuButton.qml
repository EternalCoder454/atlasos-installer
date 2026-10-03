import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// SecondaryButton with a chevron that opens a menu. Put QQC2.MenuItem children inside.
SecondaryButton {
    id: control

    default property alias items: menu.contentData

    rightPadding: control.mirrored ? leftPadding : leftPadding + Kirigami.Units.iconSizes.small
    leftPadding: control.mirrored ? Kirigami.Units.largeSpacing + Kirigami.Units.smallSpacing + Kirigami.Units.iconSizes.small : Kirigami.Units.largeSpacing + Kirigami.Units.smallSpacing
    onClicked: menu.popup(control, 0, control.height + 4)

    Kirigami.Icon {
        x: control.mirrored ? Kirigami.Units.smallSpacing + 2 : parent.width - width - Kirigami.Units.smallSpacing - 2
        anchors.verticalCenter: parent.verticalCenter
        source: "arrow-down"
        isMask: true
        color: control.textTint
        width: Kirigami.Units.iconSizes.small
        height: width
        opacity: control.enabled ? 0.8 : 0.4
    }

    QQC2.Menu {
        id: menu
        delegate: QQC2.MenuItem {
            Kirigami.MnemonicData.enabled: false
        }
    }
}
