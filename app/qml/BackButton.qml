import QtQuick
import Telamon.Ui

// A quiet Back: a ghost button with an arrow, so the page's one filled
// button is the way forward.
TelamonButton {
    text: qsTr("Back")
    variant: TelamonButton.Ghost
    symbol: LayoutMirroring.enabled ? Symbols.ArrowForward : Symbols.ArrowBack
}
