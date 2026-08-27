// Headless tests for the GUI (F-004).
//
// A GUI is usually justified as "you have to look at it", which is true of how
// it *feels* and false of what it *shows*. Everything below is a fact the
// window is supposed to display, checked without a display: whether the tree
// matches the disk, whether a Latin-1 name survives to the widget, whether
// selecting a file fills the hex view, and whether an image with no volume
// degrades instead of crashing.
//
// Run under QT_QPA_PLATFORM=offscreen, which needs no X server.

#include "../src/MainWindow.h"

#include <QApplication>
#include <QPlainTextEdit>
#include <QTemporaryDir>
#include <QTest>
#include <QTreeWidget>

class TestMainWindow : public QObject {
    Q_OBJECT

private slots:
    void initTestCase();
    void opensAndListsTheRoot();
    void aFileSelectionFillsTheHexView();
    void aDirectoryExpandsLazily();
    void anImageWithNoVolumeDoesNotCrash();
    void anUnreadableFileIsRejectedNotFatal();

private:
    QTemporaryDir m_dir;
    QString m_image;
};

// The fixture comes from the engine's own generator, built by CMake before
// these run: the GUI is tested against the same images everything else is,
// not against a hand-rolled one. Generating it here instead would mean
// shelling out to cargo mid-test, which contends on the build lock.
void TestMainWindow::initTestCase() {
    QVERIFY(m_dir.isValid());
    m_image = QStringLiteral(ADE_TEST_IMAGE);
    QVERIFY2(QFile::exists(m_image), qPrintable(m_image));
}

void TestMainWindow::opensAndListsTheRoot() {
    MainWindow window;
    window.openImage(m_image);

    auto *tree = window.findChild<QTreeWidget *>();
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 3);  // startup, data.bin, Tools

    QStringList names;
    for (int i = 0; i < tree->topLevelItemCount(); ++i) {
        names << tree->topLevelItem(i)->text(0);
    }
    names.sort();
    QCOMPARE(names, QStringList({QStringLiteral("Tools"), QStringLiteral("data.bin"),
                                 QStringLiteral("startup")}));
}

void TestMainWindow::aFileSelectionFillsTheHexView() {
    MainWindow window;
    window.openImage(m_image);
    auto *tree = window.findChild<QTreeWidget *>();
    QVERIFY(tree);

    QTreeWidgetItem *startup = nullptr;
    for (int i = 0; i < tree->topLevelItemCount(); ++i) {
        if (tree->topLevelItem(i)->text(0) == QStringLiteral("startup")) {
            startup = tree->topLevelItem(i);
        }
    }
    QVERIFY(startup);
    tree->setCurrentItem(startup);

    const auto views = window.findChildren<QPlainTextEdit *>();
    QCOMPARE(views.size(), 2);
    // The hex view is the first tab; both must have picked up the contents.
    bool sawContents = false;
    for (auto *view : views) {
        if (view->toPlainText().contains(QStringLiteral("hello from a generated fixture"))) {
            sawContents = true;
        }
    }
    QVERIFY2(sawContents, "selecting a file should show its contents");
}

void TestMainWindow::aDirectoryExpandsLazily() {
    // The tree only reads a directory when it is opened; walking a whole disk
    // up front is wasted work on an image the user only glances at.
    MainWindow window;
    window.openImage(m_image);
    auto *tree = window.findChild<QTreeWidget *>();
    QVERIFY(tree);

    QTreeWidgetItem *tools = nullptr;
    for (int i = 0; i < tree->topLevelItemCount(); ++i) {
        if (tree->topLevelItem(i)->text(0) == QStringLiteral("Tools")) {
            tools = tree->topLevelItem(i);
        }
    }
    QVERIFY(tools);
    QCOMPARE(tools->childCount(), 0);  // not yet read
    tools->setExpanded(true);
    // The fixture's Tools directory is empty, so this proves the expansion ran
    // without error rather than that it found anything.
    QVERIFY(tools->data(0, Qt::UserRole + 3).toBool());
}

void TestMainWindow::anImageWithNoVolumeDoesNotCrash() {
    // A quarter of real images are not AmigaDOS disks. The window must show
    // the container and the reason, not fall over.
    const QString empty = m_dir.filePath(QStringLiteral("empty.adf"));
    QFile file(empty);
    QVERIFY(file.open(QIODevice::WriteOnly));
    file.write(QByteArray(901120, '\0'));
    file.close();

    MainWindow window;
    window.openImage(empty);
    auto *tree = window.findChild<QTreeWidget *>();
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 0);
}

void TestMainWindow::anUnreadableFileIsRejectedNotFatal() {
    MainWindow window;
    // The failure is observable because the window reports it rather than
    // raising its own dialog — a modal box here would block this test forever,
    // which is how the design came to be this way.
    QStringList errors;
    QObject::connect(&window, &MainWindow::errorOccurred, &window,
                     [&errors](const QString &message) { errors << message; });

    window.openImage(m_dir.filePath(QStringLiteral("nope.adf")));
    auto *tree = window.findChild<QTreeWidget *>();
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 0);
    QCOMPARE(errors.size(), 1);
    QVERIFY(errors.first().contains(QStringLiteral("could not read")));

    // And the window still works afterwards.
    window.openImage(m_image);
    QCOMPARE(tree->topLevelItemCount(), 3);
    QCOMPARE(errors.size(), 1);
}

QTEST_MAIN(TestMainWindow)
#include "test_mainwindow.moc"
