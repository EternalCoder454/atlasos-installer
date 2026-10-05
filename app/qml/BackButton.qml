import QtQuick
import Atlas.Ui

// A quiet Back: a ghost button with an arrow, so the page's one filled
// button is the way forward.
AtlasButton {
    text: qsTr("Back")
    variant: AtlasButton.Ghost
    symbol: LayoutMirroring.enabled ? Symbols.ArrowForward : Symbols.ArrowBack
}
