pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui
import net.eterneon.atlas.installer

// Step 1: the AtlasOS logo, a welcome, and the installed system's language.
InstallerPage {
    id: page

    required property var app

    title: qsTr("Welcome to AtlasOS")
    subtitle: qsTr("Choose the language for AtlasOS. The installer itself is in English.")
    backVisible: false
    primaryEnabled: page.app.language.length > 0
    onPrimary: page.app.next()

    header: Image {
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        source: "qrc:/qt/qml/net/eterneon/atlas/installer/data/atlasos-logo.svg"
        sourceSize.width: Kirigami.Units.gridUnit * 4.5
        sourceSize.height: Kirigami.Units.gridUnit * 4.5
        Accessible.ignored: true
    }

    // [{code, native, english}], sorted by the English name.
    readonly property var languages: {
        const codes = JSON.parse(page.app.backend.languagesJson);
        const out = codes.map(c => {
            const d = LocaleInfo.describe(c);
            return { code: c, native: d.native, english: d.english };
        });
        out.sort((a, b) => a.english.localeCompare(b.english));
        return out;
    }
    readonly property var filtered: {
        const q = search.text.trim().toLowerCase();
        if (q.length === 0) {
            return page.languages;
        }
        return page.languages.filter(l => l.native.toLowerCase().includes(q) || l.english.toLowerCase().includes(q) || l.code.toLowerCase().startsWith(q));
    }

    function choose(code) {
        page.app.language = code;
        if (!page.app.keymapChosen) {
            page.app.keymap = page.app.backend.defaultLayout(code);
        }
    }

    Component.onCompleted: {
        search.forceActiveFocus();
        Qt.callLater(() => {
            const i = page.filtered.findIndex(l => l.code === page.app.language);
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
            placeholderText: qsTr("Search languages")
            onDown: frame.list.forceActiveFocus()
            onAccepted: {
                if (page.filtered.length > 0) {
                    page.choose(page.filtered[0].code);
                }
            }
        }

        ListFrame {
            id: frame
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: page.filtered
            emptyText: qsTr("No language matches “%1”.").arg(search.text)
            delegate: ListRow {
                required property var modelData
                title: modelData.native
                subtitle: modelData.english !== modelData.native ? modelData.english : ""
                selected: modelData.code === page.app.language
                onClicked: page.choose(modelData.code)
            }
        }
    }
}
