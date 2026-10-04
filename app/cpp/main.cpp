// Thin C++ glue: start Qt, hand the Rust backend to QML, load the window.
#include <QApplication>
#include <QCommandLineParser>
#include <QEvent>
#include <QStyleHints>
#include <QTimer>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QVariant>

#include <memory>

// Defined in src/lib.rs.
extern "C" void *atlas_backend_new();
// Defined in src/demo.rs: the demo step, empty unless demo mode is allowed.
extern "C" const char *atlas_demo_page();

namespace
{
// A focused search field's cursor blinks, and every blink is a repaint: the
// Welcome page cost about 1.5% CPU forever while the window sat open. Like
// GTK, stop blinking after a while without input (the cursor stays shown),
// and blink again on the next key or click.
class CursorBlinkLimit : public QObject
{
public:
    explicit CursorBlinkLimit(QObject *parent)
        : QObject(parent)
        , m_flashTime(QGuiApplication::styleHints()->cursorFlashTime())
    {
        m_idle.setSingleShot(true);
        m_idle.setInterval(10000);
        connect(&m_idle, &QTimer::timeout, this, [] {
            QGuiApplication::styleHints()->setCursorFlashTime(0);
        });
        m_idle.start();
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        switch (event->type()) {
        case QEvent::KeyPress:
        case QEvent::MouseButtonPress:
        case QEvent::InputMethod:
        case QEvent::FocusIn:
            if (QGuiApplication::styleHints()->cursorFlashTime() != m_flashTime) {
                QGuiApplication::styleHints()->setCursorFlashTime(m_flashTime);
            }
            m_idle.start();
            break;
        default:
            break;
        }
        return QObject::eventFilter(watched, event);
    }

private:
    const int m_flashTime;
    QTimer m_idle;
};
} // namespace

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

    // Nothing to limit when the desktop turned blinking off already.
    if (QGuiApplication::styleHints()->cursorFlashTime() > 0) {
        app.installEventFilter(new CursorBlinkLimit(&app));
    }

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
        {QStringLiteral("demoPage"), QString::fromUtf8(atlas_demo_page())},
    });
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule(QStringLiteral("net.eterneon.atlas.installer"), QStringLiteral("Main"));

    return app.exec();
}
