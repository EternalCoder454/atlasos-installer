// Language names for the Welcome page, from Qt's CLDR data: "Deutsch
// (Deutschland)" and "German (Germany)" for de_DE.UTF-8.
#pragma once

#include <QObject>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

class LocaleInfo : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

public:
    using QObject::QObject;

    // {native, english}; both fall back to the code when Qt doesn't know it.
    Q_INVOKABLE QVariantMap describe(const QString &code) const;
};
