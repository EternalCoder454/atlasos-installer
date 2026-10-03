#include "localeinfo.h"

#include <QLocale>

QVariantMap LocaleInfo::describe(const QString &code) const
{
    // "sr_RS.UTF-8@latin" -> "sr_RS" and "latin"
    QString base = code.section(QLatin1Char('.'), 0, 0).section(QLatin1Char('@'), 0, 0);
    const QString modifier = code.section(QLatin1Char('@'), 1);
    QString name = base;
    if (modifier == QLatin1String("latin")) {
        name = base.section(QLatin1Char('_'), 0, 0) + QStringLiteral("_Latn_") + base.section(QLatin1Char('_'), 1);
    }
    const QLocale l(name);
    if (l.language() == QLocale::C || l.language() == QLocale::AnyLanguage
        || base.section(QLatin1Char('_'), 0, 0) != QLocale::languageToCode(l.language(), QLocale::ISO639Part1 | QLocale::ISO639Part2 | QLocale::ISO639Part3)) {
        // Qt fell back to another language: show the code.
        return {{QStringLiteral("native"), code}, {QStringLiteral("english"), code}};
    }
    const bool hasTerritory = base.contains(QLatin1Char('_'));
    QString native = l.nativeLanguageName();
    if (!native.isEmpty()) {
        native[0] = native[0].toUpper();
    }
    QString english = QLocale::languageToString(l.language());
    if (hasTerritory) {
        native += QStringLiteral(" (") + l.nativeTerritoryName() + QLatin1Char(')');
        english += QStringLiteral(" (") + QLocale::territoryToString(l.territory()) + QLatin1Char(')');
    }
    if (!modifier.isEmpty() && modifier != QLatin1String("latin")) {
        native += QStringLiteral(", ") + modifier;
        english += QStringLiteral(", ") + modifier;
    }
    if (native.isEmpty()) {
        native = english;
    }
    return {{QStringLiteral("native"), native}, {QStringLiteral("english"), english}};
}
