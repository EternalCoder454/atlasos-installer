pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 2: the keyboard layout, a variant if it has one, and a field to try
// it in. The layout switches in the live session at once.
InstallerPage {
    id: page

    required property var app

    title: qsTr("Is this the right keyboard layout?")
    subtitle: qsTr("If not, choose yours. It works here right away, so you can try it.")
    onPrimary: page.app.next()
    onBack: page.app.previous()

    readonly property var layouts: JSON.parse(page.app.backend.layoutsJson)
    // "de(nodeadkeys)" → "de" and "nodeadkeys"
    readonly property string layoutId: page.app.keymap.split("(")[0]
    readonly property string variantId: page.app.keymap.includes("(") ? page.app.keymap.split("(")[1].replace(")", "") : ""
    readonly property var layout: page.layouts.find(l => l.id === page.layoutId) || null
    readonly property var filtered: {
        const q = search.query.trim().toLowerCase();
        if (q.length === 0) {
            return page.layouts;
        }
        return page.layouts.filter(l => l.name.toLowerCase().includes(q) || l.id === q || l.variants.some(v => v.name.toLowerCase().includes(q)));
    }

    function set(keymap) {
        page.app.keymap = keymap;
        page.app.keymapChosen = true;
        page.app.backend.applyKeymap(keymap);
    }

    Component.onCompleted: {
        page.app.backend.applyKeymap(page.app.keymap);
        search.forceActiveFocus();
        Qt.callLater(() => {
            const i = page.filtered.findIndex(l => l.id === page.layoutId);
            if (i >= 0) {
                frame.currentIndex = i;
                frame.positionAt(i);
            }
        });
    }

    ColumnLayout {
        width: page.contentWidth
        height: parent.height - Kirigami.Units.gridUnit
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Kirigami.Units.largeSpacing

        SearchField {
            id: search
            Layout.fillWidth: true
            placeholderText: qsTr("Search layouts")
            Keys.onDownPressed: frame.list.forceActiveFocus()
            onAccepted: {
                // Enter before the pause ends filters on what was typed.
                search.query = search.text;
                if (page.filtered.length > 0) {
                    page.set(page.filtered[0].id);
                }
            }
        }

        ListFrame {
            id: frame
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: page.filtered
            emptyText: qsTr("No layout matches “%1”.").arg(search.text)
            delegate: ListRow {
                required property var modelData
                title: modelData.name
                selected: modelData.id === page.layoutId
                onClicked: page.set(modelData.id)
            }
        }

        Section {
            SectionRow {
                visible: page.layout !== null && page.layout.variants.length > 0
                title: qsTr("Variant")
                AtlasComboBox {
                    id: variants
                    width: Kirigami.Units.gridUnit * 16
                    textRole: "name"
                    valueRole: "id"
                    model: page.layout ? [{ id: "", name: page.layout.name }].concat(page.layout.variants) : []
                    currentIndex: Math.max(0, model.findIndex(v => v.id === page.variantId))
                    Accessible.name: qsTr("Variant")
                    onActivated: index => {
                        const v = model[index].id;
                        page.set(v.length > 0 ? page.layoutId + "(" + v + ")" : page.layoutId);
                    }
                }
            }
            SectionRow {
                title: qsTr("Try it")
                AtlasTextField {
                    width: Kirigami.Units.gridUnit * 16
                    placeholderText: qsTr("Type here to test")
                    Accessible.name: placeholderText
                }
            }
        }
    }
}
