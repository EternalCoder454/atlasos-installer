pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui
import net.eterneon.telamon.installer

// Step 1: a welcome, and the installed system's language.
InstallerPage {
    id: page

    required property var app

    title: qsTr("Welcome to Telamon OS")
    subtitle: qsTr("Which language should Telamon OS use? The installer itself stays in English.")
    backVisible: false
    primaryEnabled: page.app.language.length > 0
    onPrimary: page.app.next()
    secondaryText: qsTr("Quick Install")
    secondaryFilled: true
    secondaryEnabled: page.app.quickReady && page.app.language.length > 0
    onSecondary: page.app.quickInstall()

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
        const q = search.query.trim().toLowerCase();
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
        Qt.callLater(page.showChosen);
    }

    function showChosen() {
        const i = page.filtered.findIndex(l => l.code === page.app.language);
        if (i >= 0) {
            frame.currentIndex = i;
            frame.positionAt(i);
        }
    }

    ColumnLayout {
        width: page.contentWidth
        height: parent.height - Kirigami.Units.gridUnit
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Kirigami.Units.largeSpacing

        SearchField {
            id: search
            Layout.fillWidth: true
            // Cleared (Escape, the clear button): back to the chosen row.
            onQueryChanged: {
                if (search.query.length === 0) {
                    Qt.callLater(page.showChosen);
                }
            }
            placeholderText: qsTr("Search languages")
            Keys.onDownPressed: frame.list.forceActiveFocus()
            onAccepted: {
                // Enter before the pause ends filters on what was typed.
                search.query = search.text;
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
