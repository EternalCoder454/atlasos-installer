// Thin C++ glue: start Qt, hand the Rust backend to QML, load the window.
#include <QApplication>
#include <QCommandLineParser>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QVariant>

#include <memory>

// Defined in src/lib.rs.
extern "C" void *atlas_backend_new();

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("atlas-installer"));
    QApplication::setApplicationDisplayName(QStringLiteral("Install AtlasOS"));
    QApplication::setApplicationVersion(QStringLiteral(ATLAS_INSTALLER_VERSION));
    QApplication::setDesktopFileName(QStringLiteral("net.eterneon.atlas.installer"));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral("Installs AtlasOS from the live session."));
    parser.addHelpOption();
    parser.addVersionOption();
    const QCommandLineOption fullScreen(QStringLiteral("fullscreen"), QStringLiteral("Fill the screen (the live installer session)."));
    parser.addOption(fullScreen);
    parser.process(app);

    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }

    // Declared before the engine, so it outlives the QML that binds to it.
    const std::unique_ptr<QObject> backend(static_cast<QObject *>(atlas_backend_new()));

    QQmlApplicationEngine engine;
    engine.setInitialProperties({
        {QStringLiteral("backend"), QVariant::fromValue(backend.get())},
        {QStringLiteral("fullScreen"), parser.isSet(fullScreen)},
        // Demo mode only: open at this step, for screenshots.
        {QStringLiteral("demoPage"), qEnvironmentVariableIsEmpty("ATLAS_INSTALLER_DEMO") ? QString() : qEnvironmentVariable("ATLAS_INSTALLER_DEMO_PAGE")},
    });
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule(QStringLiteral("net.eterneon.atlas.installer"), QStringLiteral("Main"));

    return app.exec();
}
