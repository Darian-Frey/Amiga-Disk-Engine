// Headless tests for the GUI (F-004).
//
// A GUI is usually justified as "you have to look at it", which is true of how
// it *feels* and false of what it *shows*. Everything below is a fact the
// window is supposed to display, checked without a display: whether the tree
// matches the disk, whether a Latin-1 name survives to the widget, whether
// selecting a file fills the hex view, whether search reaches every open
// image, whether a drag carries real bytes, and whether an image with no
// volume degrades instead of crashing.
//
// Run under QT_QPA_PLATFORM=offscreen, which needs no X server.

#include "../src/ImageTree.h"
#include "../src/MainWindow.h"

#include <QApplication>
#include <QLineEdit>
#include <QMimeData>
#include <QPlainTextEdit>
#include <QFontMetrics>
#include <QTemporaryDir>
#include <QTest>
#include <QTreeWidget>

namespace {

// The tree of open images, and the search results. Both are trees, so they are
// told apart by name rather than by order.
ImageTree *browser(MainWindow &window) {
    return window.findChild<ImageTree *>(QStringLiteral("tree"));
}
ImageTree *results(MainWindow &window) {
    return window.findChild<ImageTree *>(QStringLiteral("results"));
}

// An image is a root in the tree; its files hang beneath it.
QTreeWidgetItem *childNamed(QTreeWidgetItem *parent, const QString &name) {
    for (int i = 0; i < parent->childCount(); ++i) {
        if (parent->child(i)->text(0) == name) return parent->child(i);
    }
    return nullptr;
}

}  // namespace

class TestMainWindow : public QObject {
    Q_OBJECT

private slots:
    void initTestCase();
    void anImageIsARootWithItsEntriesBeneath();
    void aFileSelectionFillsTheHexView();
    void aDirectoryExpandsLazily();
    void aSecondImageDoesNotDisplaceTheFirst();
    void searchReachesEveryOpenImage();
    void searchReportsThePathAndTheImage();
    void searchMatchesNothingWithoutComplaint();
    void draggingAFileOutCarriesItsBytes();
    void draggingADirectoryCarriesNothing();
    void closingForgetsEveryImage();
    void aHardDiskShowsItsPartitionsAsALevelOfTheTree();
    void aFileInsideAPartitionPreviewsAndExtracts();
    void searchCoversEveryPartitionNotJustTheFirst();
    void anImageIsNamedFromTheDatasetAsItOpens();
    void withNoDatasetAnImageIsSimplyUnnamed();
    void theFixedColumnsShowTheirWholeContents();
    void anImageWithNoVolumeDoesNotCrash();
    void anUnreadableFileIsRejectedNotFatal();

private:
    QTemporaryDir m_dir;
    QString m_image;
    QString m_device;
};

// The fixture comes from the engine's own generator, built by CMake before
// these run: the GUI is tested against the same images everything else is,
// not against a hand-rolled one. Generating it here instead would mean
// shelling out to cargo mid-test, which contends on the build lock.
void TestMainWindow::initTestCase() {
    QVERIFY(m_dir.isValid());
    m_image = QStringLiteral(ADE_TEST_IMAGE);
    QVERIFY2(QFile::exists(m_image), qPrintable(m_image));
    m_device = QStringLiteral(ADE_TEST_DEVICE);
    QVERIFY2(QFile::exists(m_device), qPrintable(m_device));
}

void TestMainWindow::anImageIsARootWithItsEntriesBeneath() {
    MainWindow window;
    window.openImage(m_image);

    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 1);
    QTreeWidgetItem *root = tree->topLevelItem(0);
    // The image row spans: an image has no size, datestamp or protection
    // bits, so it says what it does have — the file, the container, the
    // volume — rather than borrowing a file's columns.
    QVERIFY(root->isFirstColumnSpanned());
    QVERIFY(root->text(0).startsWith(QFileInfo(m_image).fileName()));
    QVERIFY(root->text(0).contains(QStringLiteral("ADF")));
    QVERIFY(root->text(1).isEmpty());
    QCOMPARE(root->childCount(), 3);  // startup, data.bin, Tools

    QStringList names;
    for (int i = 0; i < root->childCount(); ++i) names << root->child(i)->text(0);
    names.sort();
    QCOMPARE(names, QStringList({QStringLiteral("Tools"), QStringLiteral("data.bin"),
                                 QStringLiteral("startup")}));
}

void TestMainWindow::aFileSelectionFillsTheHexView() {
    MainWindow window;
    window.openImage(m_image);
    auto *tree = browser(window);
    QVERIFY(tree);

    QTreeWidgetItem *startup = childNamed(tree->topLevelItem(0), QStringLiteral("startup"));
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
    auto *tree = browser(window);
    QVERIFY(tree);

    QTreeWidgetItem *tools = childNamed(tree->topLevelItem(0), QStringLiteral("Tools"));
    QVERIFY(tools);
    QCOMPARE(tools->childCount(), 0);  // not yet read
    tools->setExpanded(true);
    // The fixture's Tools directory is empty, so this proves the expansion ran
    // without error rather than that it found anything.
    QVERIFY(tools->data(0, Qt::UserRole + 3).toBool());
}

void TestMainWindow::aSecondImageDoesNotDisplaceTheFirst() {
    // Opening used to mean replacing. It cannot, now: search across images is
    // only meaningful if more than one can be open at a time.
    const QString copy = m_dir.filePath(QStringLiteral("second.adf"));
    QVERIFY(QFile::copy(m_image, copy));

    MainWindow window;
    window.openImage(m_image);
    window.openImage(copy);

    QCOMPARE(window.imageCount(), size_t{2});
    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 2);
    // In the order they were opened, not in alphabetical order: sorting the
    // whole tree would shuffle the images themselves, and "which did I open
    // second" is a question the window should not answer wrongly.
    QVERIFY(tree->topLevelItem(0)->text(0).startsWith(QFileInfo(m_image).fileName()));
    QVERIFY(tree->topLevelItem(1)->text(0).startsWith(QStringLiteral("second.adf")));
    QCOMPARE(tree->topLevelItem(0)->childCount(), 3);
    QCOMPARE(tree->topLevelItem(1)->childCount(), 3);
}

void TestMainWindow::searchReachesEveryOpenImage() {
    const QString copy = m_dir.filePath(QStringLiteral("also.adf"));
    QVERIFY(QFile::copy(m_image, copy));

    MainWindow window;
    window.openImage(m_image);
    window.openImage(copy);

    auto *query = window.findChild<QLineEdit *>();
    QVERIFY(query);
    query->setText(QStringLiteral("startup"));
    QMetaObject::invokeMethod(query, "returnPressed");

    auto *found = results(window);
    QVERIFY(found);
    // Both copies hold it, and both are open, so both must be reported —
    // searching only the selected image would find one.
    QCOMPARE(found->topLevelItemCount(), 2);
    QStringList images;
    for (int i = 0; i < found->topLevelItemCount(); ++i) {
        QCOMPARE(found->topLevelItem(i)->text(0), QStringLiteral("startup"));
        images << found->topLevelItem(i)->text(2);
    }
    images.sort();
    QCOMPARE(images, QStringList({QStringLiteral("also.adf"),
                                  QFileInfo(m_image).fileName()}));
}

void TestMainWindow::searchReportsThePathAndTheImage() {
    // A bare name is not enough to act on: the same name occurs on many disks
    // and in many drawers, so a result says where it lives.
    MainWindow window;
    window.openImage(m_image);

    auto *query = window.findChild<QLineEdit *>();
    QVERIFY(query);
    query->setText(QStringLiteral("data"));
    QMetaObject::invokeMethod(query, "returnPressed");

    auto *found = results(window);
    QVERIFY(found);
    QCOMPARE(found->topLevelItemCount(), 1);
    QCOMPARE(found->topLevelItem(0)->text(0), QStringLiteral("data.bin"));
    // Paths are relative to the volume root, as everywhere else in ADE.
    QCOMPARE(found->topLevelItem(0)->text(1), QStringLiteral("data.bin"));
    QCOMPARE(found->topLevelItem(0)->text(2), QFileInfo(m_image).fileName());

    // And a result is as usable as a tree row: selecting one previews it.
    auto *tree = browser(window);
    tree->setCurrentItem(childNamed(tree->topLevelItem(0), QStringLiteral("startup")));
    found->setCurrentItem(found->topLevelItem(0));
    const auto views = window.findChildren<QPlainTextEdit *>();
    QCOMPARE(views.size(), 2);
    QVERIFY(!views.first()->toPlainText().isEmpty());
    // Taking the selection means taking it: a row left highlighted in the
    // tree would read as the thing being previewed, and it is not.
    QVERIFY(tree->selectedItems().isEmpty());
    QCOMPARE(found->selectedItems().size(), 1);

    // And back the other way.
    tree->setCurrentItem(childNamed(tree->topLevelItem(0), QStringLiteral("startup")));
    QVERIFY(found->selectedItems().isEmpty());
    QCOMPARE(tree->selectedItems().size(), 1);
}

void TestMainWindow::searchMatchesNothingWithoutComplaint() {
    MainWindow window;
    window.openImage(m_image);

    auto *query = window.findChild<QLineEdit *>();
    QVERIFY(query);
    query->setText(QStringLiteral("no-such-file-anywhere"));
    QMetaObject::invokeMethod(query, "returnPressed");

    auto *found = results(window);
    QVERIFY(found);
    QCOMPARE(found->topLevelItemCount(), 0);
}

void TestMainWindow::draggingAFileOutCarriesItsBytes() {
    // Dragging to a file manager means offering a URL, which means the bytes
    // must already be on disk. This checks the file is really written, not
    // just that a URL was produced.
    MainWindow window;
    window.openImage(m_image);
    auto *tree = browser(window);
    QVERIFY(tree);

    QTreeWidgetItem *startup = childNamed(tree->topLevelItem(0), QStringLiteral("startup"));
    QVERIFY(startup);

    QScopedPointer<QMimeData> mime(tree->mimeData({startup}));
    QVERIFY(mime);
    QCOMPARE(mime->urls().size(), 1);

    const QString dragged = mime->urls().first().toLocalFile();
    QVERIFY2(QFile::exists(dragged), qPrintable(dragged));
    QCOMPARE(QFileInfo(dragged).fileName(), QStringLiteral("startup"));

    QFile out(dragged);
    QVERIFY(out.open(QIODevice::ReadOnly));
    QVERIFY(out.readAll().contains("hello from a generated fixture"));
}

void TestMainWindow::draggingADirectoryCarriesNothing() {
    // A directory has no bytes to hand over. Writing a zero-byte file named
    // after it would be worse than refusing.
    MainWindow window;
    window.openImage(m_image);
    auto *tree = browser(window);
    QVERIFY(tree);

    QTreeWidgetItem *tools = childNamed(tree->topLevelItem(0), QStringLiteral("Tools"));
    QVERIFY(tools);
    QScopedPointer<QMimeData> mime(tree->mimeData({tools}));
    QVERIFY(!mime);
}

void TestMainWindow::closingForgetsEveryImage() {
    MainWindow window;
    window.openImage(m_image);
    window.openImage(m_image);
    QCOMPARE(window.imageCount(), size_t{2});

    window.closeAll();
    QCOMPARE(window.imageCount(), size_t{0});
    QCOMPARE(browser(window)->topLevelItemCount(), 0);
    QCOMPARE(results(window)->topLevelItemCount(), 0);

    // Searching with nothing open is a no-op, not a crash.
    auto *query = window.findChild<QLineEdit *>();
    QVERIFY(query);
    query->setText(QStringLiteral("startup"));
    QMetaObject::invokeMethod(query, "returnPressed");
    QCOMPARE(results(window)->topLevelItemCount(), 0);
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
    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 1);
    QCOMPARE(tree->topLevelItem(0)->childCount(), 0);
    // The row still says what the file is and why nothing is under it.
    QVERIFY(tree->topLevelItem(0)->text(0).contains(QStringLiteral("empty.adf")));
    QVERIFY(tree->topLevelItem(0)->text(0).split(QStringLiteral("   ")).size() >= 3);

    // And searching it finds nothing rather than failing.
    auto *query = window.findChild<QLineEdit *>();
    QVERIFY(query);
    query->setText(QStringLiteral("anything"));
    QMetaObject::invokeMethod(query, "returnPressed");
    QCOMPARE(results(window)->topLevelItemCount(), 0);
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
    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 0);
    QCOMPARE(errors.size(), 1);
    QVERIFY(errors.first().contains(QStringLiteral("could not read")));

    // And the window still works afterwards.
    window.openImage(m_image);
    QCOMPARE(tree->topLevelItemCount(), 1);
    QCOMPARE(tree->topLevelItem(0)->childCount(), 3);
    QCOMPARE(errors.size(), 1);
}

void TestMainWindow::aHardDiskShowsItsPartitionsAsALevelOfTheTree() {
    // A device holds no volume of its own — every volume is inside a
    // partition. Showing the disk's files directly would mean choosing one
    // partition silently, and showing nothing would call a sound disk empty.
    MainWindow window;
    window.openImage(m_device);
    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 1);

    QTreeWidgetItem *root = tree->topLevelItem(0);
    QCOMPARE(root->childCount(), 2);
    QVERIFY(root->child(0)->text(0).startsWith(QStringLiteral("DH0")));
    QVERIFY(root->child(1)->text(0).startsWith(QStringLiteral("DH1")));
    // Each partition row says what it is, spanning the columns for the same
    // reason an image row does: a partition has no size or protection bits.
    QVERIFY(root->child(0)->isFirstColumnSpanned());
    QVERIFY(root->child(0)->text(0).contains(QStringLiteral("bootable")));
    QVERIFY(!root->child(1)->text(0).contains(QStringLiteral("bootable")));

    // And the files are under the partition that holds them.
    QVERIFY(childNamed(root->child(0), QStringLiteral("startup-sequence")));
    QVERIFY(childNamed(root->child(0), QStringLiteral("Tools")));
    QVERIFY(childNamed(root->child(1), QStringLiteral("data.bin")));
    QVERIFY2(!childNamed(root->child(1), QStringLiteral("startup-sequence")),
             "a partition must not show another partition's files");
}

void TestMainWindow::aFileInsideAPartitionPreviewsAndExtracts() {
    // The block numbers inside two partitions overlap, so reading one with the
    // other's volume would silently return the wrong file rather than fail.
    MainWindow window;
    window.openImage(m_device);
    auto *tree = browser(window);
    QVERIFY(tree);
    QTreeWidgetItem *root = tree->topLevelItem(0);

    QTreeWidgetItem *startup =
        childNamed(root->child(0), QStringLiteral("startup-sequence"));
    QVERIFY(startup);
    tree->setCurrentItem(startup);

    const auto views = window.findChildren<QPlainTextEdit *>();
    QCOMPARE(views.size(), 2);
    bool sawContents = false;
    for (auto *view : views) {
        if (view->toPlainText().contains(QStringLiteral("hello from DH0"))) sawContents = true;
    }
    QVERIFY2(sawContents, "a file inside a partition should preview");

    // And drag out, which reads it a second way.
    QScopedPointer<QMimeData> mime(tree->mimeData({startup}));
    QVERIFY(mime);
    QCOMPARE(mime->urls().size(), 1);
    QFile out(mime->urls().first().toLocalFile());
    QVERIFY(out.open(QIODevice::ReadOnly));
    QVERIFY(out.readAll().contains("hello from DH0"));
}

void TestMainWindow::searchCoversEveryPartitionNotJustTheFirst() {
    MainWindow window;
    window.openImage(m_device);

    auto *query = window.findChild<QLineEdit *>();
    QVERIFY(query);
    // `readme` exists in **both** partitions. Searching only the first volume
    // — which is what a device did before partitions existed here — finds one
    // of the two and looks perfectly successful.
    query->setText(QStringLiteral("readme"));
    QMetaObject::invokeMethod(query, "returnPressed");

    auto *found = results(window);
    QVERIFY(found);
    QCOMPARE(found->topLevelItemCount(), 2);
    QStringList where;
    for (int i = 0; i < found->topLevelItemCount(); ++i) {
        QCOMPARE(found->topLevelItem(i)->text(0), QStringLiteral("readme"));
        where << found->topLevelItem(i)->text(2);
    }
    QVERIFY2(where.filter(QStringLiteral("DH0")).size() > 0, "DH0 should be searched");
    QVERIFY2(where.filter(QStringLiteral("DH1")).size() > 0, "DH1 should be searched too");
    // The result says which partition, not merely which file: the same name
    // occurs in more than one volume of one disk.
    QVERIFY(where.first().contains(QStringLiteral("—")));

    // And selecting each gives the file from *that* partition. The two share a
    // name and a path, so nothing but the partition tells them apart.
    const auto views = window.findChildren<QPlainTextEdit *>();
    QStringList shown;
    for (int i = 0; i < 2; ++i) {
        found->setCurrentItem(found->topLevelItem(i));
        shown << views.at(1)->toPlainText();
    }
    QVERIFY2(shown[0] != shown[1], "two files of one name must read differently");
    QVERIFY(shown.filter(QStringLiteral("this is DH0")).size() == 1);
    QVERIFY(shown.filter(QStringLiteral("this is DH1")).size() == 1);
}

void TestMainWindow::anImageIsNamedFromTheDatasetAsItOpens() {
    // F-013's clause where it pays: the dataset loads once for the session and
    // every image opened afterwards arrives already named. The window reads
    // $ADE_DATFILES, and CMake generates a dataset matching the fixture —
    // computing the CRC32 here would reimplement the thing under test.
    const QString datfiles = QStringLiteral(ADE_TEST_DATFILES);
    QVERIFY2(QFile::exists(datfiles + QStringLiteral("/fixture.dat")), qPrintable(datfiles));

    qputenv("ADE_DATFILES", datfiles.toUtf8());
    MainWindow window;
    window.openImage(m_image);
    qunsetenv("ADE_DATFILES");

    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 1);
    QVERIFY2(tree->topLevelItem(0)->toolTip(0).contains(QStringLiteral("A Named Disk.adf")),
             qPrintable(tree->topLevelItem(0)->toolTip(0)));
}

void TestMainWindow::withNoDatasetAnImageIsSimplyUnnamed() {
    // The ordinary case, and it must cost nothing: no dataset configured, no
    // identification, no complaint.
    qunsetenv("ADE_DATFILES");
    MainWindow window;
    window.openImage(m_image);
    auto *tree = browser(window);
    QVERIFY(tree);
    QVERIFY(!tree->topLevelItem(0)->toolTip(0).contains(QStringLiteral("A Named Disk.adf")));
}

void TestMainWindow::theFixedColumnsShowTheirWholeContents() {
    // Two things at once, and the second is why this is the guard.
    //
    // Left to stretch, the Modified column truncated to "1990-09-20 17:..." —
    // the one part of a timestamp nobody can infer. So the widths are set from
    // the *widest value each column can hold*, measured once.
    //
    // The obvious way to do that is `QHeaderView::ResizeToContents`, which is
    // quadratic: Qt re-measures every row in the tree on every insertion, and
    // expanding 3,827 rows took 35.6 seconds against 0.25 (IMP-008). This test
    // catches its return deterministically, where a timing test cannot: that
    // mode sizes to the content actually present, and a fixture's short names
    // and small sizes are narrower than the widest value the column must fit.
    // Checked by reintroducing it — this fails, and a timing assertion at
    // fixture scale does not.
    MainWindow window;
    window.openImage(m_image);
    auto *tree = browser(window);
    QVERIFY(tree);

    const QFontMetrics metrics(tree->font());
    QVERIFY2(tree->columnWidth(2) >= metrics.horizontalAdvance(QStringLiteral("1990-09-20 17:10:20")),
             "a full datestamp must fit");
    QVERIFY2(tree->columnWidth(3) >= metrics.horizontalAdvance(QStringLiteral("hsparwed")),
             "all eight protection flags must fit");
    QVERIFY2(tree->columnWidth(1) >= metrics.horizontalAdvance(QStringLiteral("999999999")),
             "the largest size on a floppy must fit");
}

QTEST_MAIN(TestMainWindow)
#include "test_mainwindow.moc"
