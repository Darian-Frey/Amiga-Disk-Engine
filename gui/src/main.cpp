// The Qt6 front end (F-004).
//
// Nothing here knows anything about Amiga filesystems: every fact on screen
// came through the C ABI from the engine (F-002, D-001). That is the whole
// point of the split — the GUI cannot drift from the CLI, because there is
// only one implementation underneath both.

#include "MainWindow.h"

#include <QApplication>
#include <QMessageBox>

int main(int argc, char *argv[]) {
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Amiga Disk Engine"));

    MainWindow window;
    // The window reports failures; the application decides they are dialogs.
    // Keeping that decision here is what leaves every failure path testable.
    QObject::connect(&window, &MainWindow::errorOccurred, &window,
                     [&window](const QString &message) {
                         QMessageBox::warning(&window, QStringLiteral("Amiga Disk Engine"),
                                              message);
                     });
    window.show();

    // An image named on the command line opens straight away, which makes the
    // GUI usable from a file manager and testable without clicking.
    const QStringList args = QApplication::arguments();
    if (args.size() > 1) window.openImage(args.at(1));

    return QApplication::exec();
}
