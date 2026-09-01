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

#include "../src/HexView.h"
#include "../src/MapView.h"
#include "../src/ImageTree.h"
#include "../src/MainWindow.h"

#include <QApplication>
#include <QLineEdit>
#include <QMimeData>
#include <QPlainTextEdit>
#include <QFontMetrics>
#include <QTemporaryDir>
#include <QTest>
#include <QClipboard>
#include <QComboBox>
#include <QItemSelectionModel>
#include <QStatusBar>
#include <QTreeWidgetItemIterator>
#include <QDir>
#include <QTabWidget>
#include <cmath>
#include <QLabel>
#include <QMenu>
#include <QMenuBar>
#include <QScreen>
#include <QScrollBar>
#include <QTextBlock>
#include <QTextLayout>
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
    void nullBytesAreDimmedInTheHexFieldAndNowhereElse();
    void theDimColourFollowsTheTheme();
    void aDragAcrossLinesSelectsOnlyTheFieldItStartedIn();
    void aDragInTheAsciiFieldStaysInTheAsciiField();
    void aDragThatStraysIntoAnotherFieldKeepsGoingInItsOwn();
    void copyingGivesBackExactlyWhatWasHighlighted();
    void wholeLinesCanStillBeCopiedWhenThatIsWhatIsWanted();
    void selectingTheDiskRowShowsTheWholeDisk();
    void theWholeDiskViewTintsItsRegionsAndAFileViewDoesNot();
    void theLegendNamesOnlyTheRegionsTheDiskHas();
    void scrollingIntoAFileMarksItsRowWithoutSelectingIt();
    void scrollingOutOfAFileUnmarksIt();
    void aScrollQtDoesNotAnnounceIsStillFollowed();
    void theFollowStopsWhenAFileIsShown();
    void extractingEverythingIsOfferedOnlyWithADiskToExtract();
    void thereIsADiskMenuOfferedOnlyWithADiskOpen();
    void theMapColoursFilesAndKeepsEmptySpaceVisible();
    void theMapShowsTheDiskAndClickingACellGoesToIt();
    void selectingAFilePicksOutItsBlocksOnTheMap();
    void newDiskIsOfferedAndItsTypesComeFromTheEngine();
    void aFileThatIsNotADiskImageIsDeclinedRatherThanShown();
    void thereIsAHelpMenuWithAnAboutBox();
    void theAboutBoxTakesItsVersionFromTheEngine();
    void theSearchBoxCanSearchContentsInsteadOfNames();
    void aRefusedPatternSaysWhyRatherThanFindingNothing();
    void clickingAContentHitGoesToItInTheWholeDiskView();
    void theHexPaneIsGivenTheRoomADumpLineNeeds();
    void theDefaultSizeFitsADumpLineWhereTheScreenAllows();
    void selectingHexMarksTheCharactersThoseBytesSpell();
    void selectingCharactersMarksTheirHex();
    void theMarkIsWeakerThanTheSelectionSoTheCopiedFieldIsObvious();

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

// The hex field's zero bytes are dimmed so real data stands out from padding
// (the Atari Disk Engine does the same). Two things can go wrong and neither
// is visible in a screenshot: dimming the wrong columns, and dimming nothing.
void TestMainWindow::nullBytesAreDimmedInTheHexFieldAndNowhereElse() {
    QPlainTextEdit pane;
    HexHighlighter dimmer(pane.document(), &pane);

    // A line whose offset is all zeros, whose first byte is zero, whose ASCII
    // column contains the literal characters `00`, and which also holds a
    // non-zero byte. Every trap in one line.
    QByteArray data(16, '\x41');
    data[0] = '\x00';
    data[1] = '0';
    data[2] = '0';
    pane.setPlainText(hexview::dump(data));

    const QTextBlock line = pane.document()->findBlockByNumber(0);
    QVERIFY(line.isValid());
    QList<int> dimmed;
    for (const auto &range : line.layout()->formats()) {
        dimmed.append(range.start);
    }

    // The one null byte is dimmed, and it is the only thing that is.
    QCOMPARE(dimmed, QList<int>{hexview::columnOf(0)});

    // Which means, specifically:
    const QString text = line.text();
    QVERIFY2(text.startsWith(QStringLiteral("00000000  00 30 30 41")), qPrintable(text));
    QVERIFY2(!dimmed.contains(0), "the offset column is mostly zeros; dimming it "
                                  "would dim every line rather than the data");
    QVERIFY2(!dimmed.contains(text.indexOf(QStringLiteral("|"))),
             "the ASCII column can hold the characters 00 from the file itself");
    QVERIFY2(!dimmed.contains(hexview::columnOf(1)),
             "byte 1 is the character '0', which is 0x30 and not a null");
}

void TestMainWindow::theDimColourFollowsTheTheme() {
    // Dim means "between the text and the background", not a fixed grey: a
    // hardcoded light-theme grey is invisible on a dark one. Checked at both
    // ends rather than by naming a colour, so the blend can be retuned without
    // rewriting the test.
    QPalette light;
    light.setColor(QPalette::Text, Qt::black);
    light.setColor(QPalette::Base, Qt::white);
    const QColor dimLight = HexHighlighter::dimColour(light);
    QVERIFY(dimLight.lightness() > QColor(Qt::black).lightness());
    QVERIFY(dimLight.lightness() < QColor(Qt::white).lightness());

    QPalette dark;
    dark.setColor(QPalette::Text, Qt::white);
    dark.setColor(QPalette::Base, QColor(30, 30, 30));
    const QColor dimDark = HexHighlighter::dimColour(dark);
    QVERIFY2(dimDark.lightness() < dimLight.lightness(),
             "a dark theme's dim must be darker than a light theme's, not the same grey");
    QVERIFY(dimDark.lightness() > QColor(30, 30, 30).lightness());
}

// A hex dump is three columns pretending to be one line of text, and ordinary
// text selection does not know that: dragging down two lines in the hex field
// takes the end of that line's hex, the ASCII column, the next line's offset,
// and only then more hex. These pin the clamping that fixes it.
namespace {

// A pane holding `lines` dump lines of recognisable bytes: line n is n+1
// repeated, so a selection's extent is legible in the copied text.
HexPane *dumpPane(int lines) {
    QByteArray data;
    for (int line = 0; line < lines; ++line) {
        data.append(QByteArray(16, static_cast<char>(line + 1)));
    }
    auto *pane = new HexPane;
    pane->setFont(QFont(QStringLiteral("monospace"), 10));
    pane->setLineWrapMode(QPlainTextEdit::NoWrap);
    pane->setPlainText(hexview::dump(data));
    pane->resize(700, 300);
    // Shown, or the document has no layout and every point maps to the same
    // cursor — the drag would then select nothing and the test would pass or
    // fail for reasons unrelated to the clamping.
    pane->show();
    return pane;
}

// The viewport point at (line, column) of the dump.
QPoint at(HexPane *pane, int line, int column) {
    const QTextBlock block = pane->document()->findBlockByNumber(line);
    QTextCursor cursor(block);
    cursor.setPosition(block.position() + column);
    const QRect r = pane->cursorRect(cursor);
    return QPoint(r.left() + 1, r.center().y());
}

// Press at one point, drag to another, release.
void drag(HexPane *pane, QPoint from, QPoint to) {
    // To the viewport, not the frame: a scroll area's mouse events arrive
    // there, and `cursorRect` gives viewport coordinates to match.
    QMouseEvent press(QEvent::MouseButtonPress, from, pane->mapToGlobal(from), Qt::LeftButton,
                      Qt::LeftButton, Qt::NoModifier);
    QApplication::sendEvent(pane->viewport(), &press);
    QMouseEvent move(QEvent::MouseMove, to, pane->mapToGlobal(to), Qt::NoButton, Qt::LeftButton,
                     Qt::NoModifier);
    QApplication::sendEvent(pane->viewport(), &move);
    QMouseEvent release(QEvent::MouseButtonRelease, to, pane->mapToGlobal(to), Qt::LeftButton,
                        Qt::NoButton, Qt::NoModifier);
    QApplication::sendEvent(pane->viewport(), &release);
}

}  // namespace

void TestMainWindow::aDragAcrossLinesSelectsOnlyTheFieldItStartedIn() {
    QScopedPointer<HexPane> pane(dumpPane(4));
    // From byte 2 of line 0 to byte 5 of line 2, in the hex field.
    drag(pane.data(), at(pane.data(), 0, hexview::columnOf(2)),
         at(pane.data(), 2, hexview::columnOf(5)));

    const QStringList got = pane->selectedFieldText().split(QChar('\n'));
    QCOMPARE(got.size(), 3);
    // Line 0 runs from the byte grabbed to the end of the field; the middle
    // line is whole; the last stops where the pointer did.
    // Bytes 2 through 15: six before the gap that splits the two groups of
    // eight, then the last eight.
    QCOMPARE(got[0], QStringLiteral("01 01 01 01 01 01  01 01 01 01 01 01 01 01"));
    QCOMPARE(got[1], QStringLiteral("02 02 02 02 02 02 02 02  02 02 02 02 02 02 02 02"));
    QCOMPARE(got[2], QStringLiteral("03 03 03 03 03 03"));

    // Nothing from the other two fields came with it.
    const QString all = pane->selectedFieldText();
    QVERIFY2(!all.contains(QChar('|')), "the ASCII column must not be dragged in");
    QVERIFY2(!all.contains(QStringLiteral("00000010")), "nor the next line's offset");
}

void TestMainWindow::aDragInTheAsciiFieldStaysInTheAsciiField() {
    QScopedPointer<HexPane> pane(dumpPane(3));
    drag(pane.data(), at(pane.data(), 0, hexview::AsciiColumn),
         at(pane.data(), 1, hexview::AsciiColumn + 3));

    const QStringList got = pane->selectedFieldText().split(QChar('\n'));
    QCOMPARE(got.size(), 2);
    // 0x01 and 0x02 are unprintable, so the dump shows dots — sixteen of them
    // on the first line, four on the second.
    QCOMPARE(got[0], QString(16, QChar('.')));
    QCOMPARE(got[1], QString(4, QChar('.')));
}

void TestMainWindow::aDragThatStraysIntoAnotherFieldKeepsGoingInItsOwn() {
    // The pointer wandering into the ASCII column must not silently change
    // what is being selected — that is the behaviour being fixed, not a
    // feature to preserve.
    QScopedPointer<HexPane> pane(dumpPane(3));
    drag(pane.data(), at(pane.data(), 0, hexview::columnOf(0)),
         at(pane.data(), 1, hexview::AsciiColumn + 8));

    const QString got = pane->selectedFieldText();
    QVERIFY2(got.startsWith(QStringLiteral("01 01")), qPrintable(got));
    QVERIFY2(!got.contains(QChar('.')), "still hex, not the characters it strayed over");
}

void TestMainWindow::copyingGivesBackExactlyWhatWasHighlighted() {
    QScopedPointer<HexPane> pane(dumpPane(3));
    drag(pane.data(), at(pane.data(), 1, hexview::columnOf(0)),
         at(pane.data(), 1, hexview::columnOf(3)));

    QApplication::clipboard()->clear();
    QKeyEvent copy(QEvent::KeyPress, Qt::Key_C, Qt::ControlModifier);
    QApplication::sendEvent(pane.data(), &copy);
    QCOMPARE(QApplication::clipboard()->text(), QStringLiteral("02 02 02 02"));

    // And a plain click clears it, rather than leaving a one-byte remnant.
    const QPoint spot = at(pane.data(), 0, hexview::columnOf(0));
    drag(pane.data(), spot, spot);
    QVERIFY(pane->selectedFieldText().isEmpty());
}

void TestMainWindow::wholeLinesCanStillBeCopiedWhenThatIsWhatIsWanted() {
    // Clamping the drag removes the only way there was to copy a line as it
    // appears — which is what somebody pasting into a bug report wants — so
    // the context menu keeps it.
    QScopedPointer<HexPane> pane(dumpPane(3));
    drag(pane.data(), at(pane.data(), 0, hexview::columnOf(4)),
         at(pane.data(), 1, hexview::columnOf(4)));

    const QStringList got = pane->selectedLines().split(QChar('\n'));
    QCOMPARE(got.size(), 2);
    QVERIFY2(got[0].startsWith(QStringLiteral("00000000  01 01")), qPrintable(got[0]));
    QVERIFY2(got[0].endsWith(QStringLiteral("|................|")), qPrintable(got[0]));
    QVERIFY2(got[1].startsWith(QStringLiteral("00000010  02 02")), qPrintable(got[1]));
}

// Hex and characters are two readings of the same bytes, and which bytes is
// the question a hex view exists to answer — so a selection in one field marks
// the same bytes in the other.
namespace {

// Every highlighted range as (line, first column, last column), in order.
QList<std::tuple<int, int, int>> marks(HexPane *pane) {
    QList<std::tuple<int, int, int>> out;
    for (const auto &selection : pane->extraSelections()) {
        const QTextBlock block = selection.cursor.block();
        const int start = selection.cursor.selectionStart() - block.position();
        const int end = selection.cursor.selectionEnd() - block.position();
        out.append({block.blockNumber(), start, end - 1});
    }
    return out;
}

// Whether any highlight covers exactly that span, and with which background.
bool markedFrom(HexPane *pane, int line, int first, int last, QColor &colour) {
    for (const auto &selection : pane->extraSelections()) {
        const QTextBlock block = selection.cursor.block();
        if (block.blockNumber() != line) continue;
        const int start = selection.cursor.selectionStart() - block.position();
        const int end = selection.cursor.selectionEnd() - block.position() - 1;
        if (start == first && end == last) {
            colour = selection.format.background().color();
            return true;
        }
    }
    return false;
}

}  // namespace

void TestMainWindow::selectingHexMarksTheCharactersThoseBytesSpell() {
    QScopedPointer<HexPane> pane(dumpPane(3));
    // Bytes 4 through 9 of line 1, in the hex field.
    drag(pane.data(), at(pane.data(), 1, hexview::columnOf(4)),
         at(pane.data(), 1, hexview::columnOf(9)));

    QColor colour;
    QVERIFY2(markedFrom(pane.data(), 1, hexview::AsciiColumn + 4, hexview::AsciiColumn + 9, colour),
             "the same six bytes must be marked in the characters");
    QCOMPARE(colour, HexPane::mirrorColour(pane->palette()));

    // And only there: two ranges on one line, no other line touched.
    QCOMPARE(marks(pane.data()).size(), 2);
}

void TestMainWindow::selectingCharactersMarksTheirHex() {
    QScopedPointer<HexPane> pane(dumpPane(3));
    drag(pane.data(), at(pane.data(), 0, hexview::AsciiColumn + 2),
         at(pane.data(), 1, hexview::AsciiColumn + 5));

    QColor colour;
    // The first line runs from byte 2 to the end of the field; the hex mark
    // has to follow the gap that splits the two groups of eight, which is why
    // this is not simply "start plus three times the count".
    QVERIFY2(markedFrom(pane.data(), 0, hexview::columnOf(2), hexview::columnOf(15) + 1, colour),
             "the first line's hex, from the byte grabbed to the end");
    QVERIFY2(markedFrom(pane.data(), 1, hexview::columnOf(0), hexview::columnOf(5) + 1, colour),
             "the last line's hex, stopping where the pointer did");

    // What is copied is still only the characters.
    QVERIFY2(!pane->selectedFieldText().contains(QChar('0')),
             "marking hex must not put hex on the clipboard");
}

void TestMainWindow::theMarkIsWeakerThanTheSelectionSoTheCopiedFieldIsObvious() {
    // Painted alike, both fields would look equally selected and nothing on
    // screen would say which one Ctrl+C copies — a worse ambiguity than the
    // one being fixed.
    QScopedPointer<HexPane> pane(dumpPane(2));
    QPalette p = pane->palette();
    p.setColor(QPalette::Base, Qt::white);
    p.setColor(QPalette::Highlight, QColor(53, 132, 228));
    pane->setPalette(p);

    const QColor selected = p.color(QPalette::Highlight);
    const QColor marked = HexPane::mirrorColour(p);
    QVERIFY2(marked != selected, "the two must be distinguishable");
    QVERIFY2(marked.lightness() > selected.lightness(),
             "against a light background the mark is the paler of the two");
    QVERIFY2(marked != p.color(QPalette::Base), "but still visible against the page");

    // The selection keeps the highlighted-text colour; the mark leaves the
    // text alone, so a dimmed null stays dimmed under it.
    drag(pane.data(), at(pane.data(), 0, hexview::columnOf(0)),
         at(pane.data(), 0, hexview::columnOf(3)));
    for (const auto &selection : pane->extraSelections()) {
        const bool isMark = selection.format.background().color() == marked;
        QCOMPARE(selection.format.hasProperty(QTextFormat::ForegroundBrush), !isMark);
    }
}

// The window used to open at a hardcoded 1100x700 with an even split, giving
// the hex pane about 550 pixels for a line of 78 monospaced characters — so
// the characters column was cut off on the first disk anybody opened.
//
// Neither test names a pixel width. The fixed font is whatever the desktop
// calls fixed-width and is a different size on two machines: measured here a
// line wants 563px offscreen and 609px on the development display, so a test
// written around 1100x700 passes in CI and fails on the desk.
namespace {

// The pane, and what one dump line measures in its font.
QPair<QPlainTextEdit *, int> hexPaneOf(MainWindow &window) {
    auto *hex = window.findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    if (!hex) return {nullptr, 0};
    const QFontMetrics metrics(hex->font());
    return {hex, metrics.horizontalAdvance(QString(hexview::LineLength, QChar('0')))};
}

// Open the fixture and select a file, so the pane holds a real dump.
bool showADump(MainWindow &window, const QString &image) {
    window.show();
    window.openImage(image);
    auto *tree = browser(window);
    if (!tree || tree->topLevelItemCount() == 0) return false;
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("startup"));
    if (!file) return false;
    tree->setCurrentItem(file);
    QApplication::processEvents();
    return true;
}

}  // namespace

void TestMainWindow::theHexPaneIsGivenTheRoomADumpLineNeeds() {
    // Given a window with room to spare, the split must spend it on the pane
    // that has a content width. This is the half that was wrong: an even split
    // gave the tree more than it could use and the dump less than it needed.
    MainWindow window;
    const auto [hex, line] = hexPaneOf(window);
    QVERIFY(hex);
    window.resize(line + 700, 700);
    QVERIFY(showADump(window, m_image));

    QVERIFY2(hex->viewport()->width() >= line,
             qPrintable(QStringLiteral("a dump line needs %1px, the pane has %2px")
                            .arg(line)
                            .arg(hex->viewport()->width())));
    QVERIFY2(!hex->horizontalScrollBar()->isVisible(),
             "no horizontal scrollbar on a freshly opened disk");
}

void TestMainWindow::theDefaultSizeFitsADumpLineWhereTheScreenAllows() {
    // And the window must ask for that room itself, without being resized —
    // which is the actual complaint. Skipped rather than failed on a display
    // too small to hold a dump line beside a usable tree: the offscreen
    // platform reports 800x800, where no split of the width fits.
    MainWindow window;
    const auto [hex, line] = hexPaneOf(window);
    QVERIFY(hex);
    const QScreen *screen = QGuiApplication::primaryScreen();
    QVERIFY(screen);
    if (screen->availableGeometry().width() < line + 400) {
        QSKIP("this screen cannot hold a dump line beside a usable tree");
    }
    QVERIFY(showADump(window, m_image));

    QVERIFY2(hex->viewport()->width() >= line,
             qPrintable(QStringLiteral("at its own default size of %1px wide, a dump line needs "
                                       "%2px and the pane has %3px")
                            .arg(window.width())
                            .arg(line)
                            .arg(hex->viewport()->width())));
}

// Selecting the image row shows the whole disk, with its regions tinted (F-022).
// A file view can never reach the bootblock, the rootblock, the bitmap, or the
// space no directory entry points at — which on a damaged disk is where the
// interesting part is.
namespace {

// Open the fixture and select the image's own row.
MainWindow *diskWindow(const QString &image) {
    auto *window = new MainWindow;
    window->resize(1200, 700);
    window->show();
    window->openImage(image);
    auto *tree = browser(*window);
    if (tree && tree->topLevelItemCount() > 0) tree->setCurrentItem(tree->topLevelItem(0));
    QApplication::processEvents();
    return window;
}

// The background the highlighter painted on the line holding `offset`.
QColor tintAt(QPlainTextEdit *hex, int offset) {
    const QTextBlock block = hex->document()->findBlockByNumber(offset / hexview::BytesPerLine);
    if (!block.isValid()) return {};
    // Any range carrying a background. The wash is set across the whole line,
    // but Qt splits it into fragments wherever the null dimming sets a
    // foreground on top — so there is no single range spanning the line, which
    // is what this looked for first and why it found nothing.
    for (const auto &range : block.layout()->formats()) {
        if (range.format.background().style() != Qt::NoBrush) {
            return range.format.background().color();
        }
    }
    return {};
}

}  // namespace

void TestMainWindow::selectingTheDiskRowShowsTheWholeDisk() {
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *hex = window->findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    QVERIFY(hex);

    // Not a file: the first line is the disk's own byte 0, which is `DOS`.
    const QString text = hex->toPlainText();
    QVERIFY2(text.startsWith(QStringLiteral("00000000  44 4f 53")), qPrintable(text.left(40)));

    // And it is the whole disk, not a preview of the first block.
    QCOMPARE(hex->document()->blockCount() - 1, 901120 / hexview::BytesPerLine);
}

void TestMainWindow::theWholeDiskViewTintsItsRegionsAndAFileViewDoesNot() {
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *hex = window->findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    QVERIFY(hex);

    const QColor boot = tintAt(hex, 0);
    QVERIFY2(boot.isValid(), "the bootblock is tinted");
    QCOMPARE(boot, HexHighlighter::regionColour(hex->palette(), ADE_REGION_BOOTBLOCK));

    // A DD floppy's rootblock is block 880, and it is a different colour.
    const QColor root = tintAt(hex, 880 * 512);
    QVERIFY2(root.isValid(), "the rootblock is tinted");
    QVERIFY2(root != boot, "and not the same colour as the bootblock");

    // Files are deliberately not tinted: they are most of a disk, and
    // colouring everything is the same as colouring nothing.
    QVERIFY(!HexHighlighter::regionColour(hex->palette(), ADE_REGION_FILE).isValid());

    // Selecting a file afterwards clears the map. A leftover would colour the
    // file's bytes by where some other view's offsets fell — the worst kind of
    // wrong, because it looks deliberate.
    auto *tree = browser(*window);
    // Expanded first. A row inside a collapsed parent is a row nobody can
    // click, and selecting one emits no `itemSelectionChanged` — which made
    // this look like the window failing to clear its map, when it was the test
    // driving an interaction the interface cannot produce.
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("startup"));
    QVERIFY(file);
    // Selected explicitly, not just made current. The window listens for
    // `itemSelectionChanged` and reads `selectedItems()`, and plain
    // `setCurrentItem` was observed to move the current row without moving the
    // selection — after which the window redrew the row that was still
    // selected, which is the disk, and looked like a failure to clear.
    tree->setCurrentItem(file, 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();
    QVERIFY2(!tintAt(hex, 0).isValid(), "a file's own bytes are one region, so none is shown");
}

void TestMainWindow::theLegendNamesOnlyTheRegionsTheDiskHas() {
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *legend = window->findChild<QLabel *>(QStringLiteral("legend"));
    QVERIFY(legend);
    QVERIFY2(legend->isVisible(), "a colour nobody can name is decoration");

    // Named from the engine, not from strings written in Qt — the GUI knows
    // nothing about Amiga filesystems, and a legend written here would be the
    // first thing to drift from --format=json.
    for (int region : {ADE_REGION_BOOTBLOCK, ADE_REGION_ROOTBLOCK, ADE_REGION_FILE}) {
        const QString name = QString::fromUtf8(ade_region_name(static_cast<AdeRegion>(region)));
        QVERIFY2(legend->text().contains(name), qPrintable(name));
    }

    // Hidden again for a file, where there is nothing to explain.
    auto *tree = browser(*window);
    // Expanded first. A row inside a collapsed parent is a row nobody can
    // click, and selecting one emits no `itemSelectionChanged` — which made
    // this look like the window failing to clear its map, when it was the test
    // driving an interaction the interface cannot produce.
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("startup"));
    QVERIFY(file);
    // Selected explicitly, not just made current. The window listens for
    // `itemSelectionChanged` and reads `selectedItems()`, and plain
    // `setCurrentItem` was observed to move the current row without moving the
    // selection — after which the window redrew the row that was still
    // selected, which is the disk, and looked like a failure to clear.
    tree->setCurrentItem(file, 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();
    QVERIFY(!legend->isVisible());
}

// Scrolling the whole disk says which file is on screen (F-022).
namespace {

// Scroll the hex pane so `offset` is the top line.
void scrollTo(MainWindow &window, quint64 offset) {
    auto *hex = window.findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    hex->verticalScrollBar()->setValue(static_cast<int>(offset / hexview::BytesPerLine));
    QApplication::processEvents();
}

// The marked row, if any: the one shown in bold that is not an image's own
// header row (those are bold already, to separate the disks from their files).
QTreeWidgetItem *markedRow(MainWindow &window) {
    auto *tree = browser(window);
    for (QTreeWidgetItemIterator it(tree); *it; ++it) {
        if ((*it)->font(0).bold() && (*it)->data(0, tree::RoleBlock).isValid()) return *it;
    }
    return nullptr;
}

QString status(MainWindow &window) {
    auto *bar = window.findChild<QStatusBar *>();
    return bar ? bar->currentMessage() : QString{};
}

}  // namespace

void TestMainWindow::scrollingIntoAFileMarksItsRowWithoutSelectingIt() {
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *tree = browser(*window);
    tree->expandItem(tree->topLevelItem(0));

    // `data.bin` is the fixture's larger file, so it occupies a run of blocks
    // rather than a single one — the map says where.
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("data.bin"));
    QVERIFY(file);
    const quint32 block = file->data(0, tree::RoleBlock).toUInt();
    scrollTo(*window, static_cast<quint64>(block) * 512);

    QCOMPARE(markedRow(*window), file);
    QVERIFY2(status(*window).contains(QStringLiteral("data.bin")), qPrintable(status(*window)));

    // **The mark is not a selection.** Selection is what chose the whole-disk
    // view, so following the scroll with it would replace the very view being
    // scrolled — the feature would undo itself on the first wheel click.
    QCOMPARE(tree->selectedItems().size(), 1);
    QCOMPARE(tree->selectedItems().first(), tree->topLevelItem(0));
    auto *hex = window->findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    QCOMPARE(hex->document()->blockCount() - 1, 901120 / hexview::BytesPerLine);
}

void TestMainWindow::scrollingOutOfAFileUnmarksIt() {
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *tree = browser(*window);
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("data.bin"));
    QVERIFY(file);

    scrollTo(*window, static_cast<quint64>(file->data(0, tree::RoleBlock).toUInt()) * 512);
    QVERIFY(markedRow(*window));

    // Back to the bootblock, which no file owns.
    scrollTo(*window, 0);
    QVERIFY2(!markedRow(*window), "a mark left behind names the wrong file");
    QVERIFY2(status(*window).contains(QStringLiteral("bootblock")), qPrintable(status(*window)));
}

void TestMainWindow::aScrollQtDoesNotAnnounceIsStillFollowed() {
    // Qt does not reliably say when the view moved. Measured on the real
    // display: a click in the scrollbar trough scrolled the pane from line 0
    // to line 37 — the painted text proves it moved — while emitting
    // `valueChanged` zero times and calling `scrollContentsBy` zero times. The
    // wheel and a drag of the handle both notify, so the gap reads as the
    // feature working right up until somebody pages down.
    //
    // Blocking the signals here reproduces that shape without depending on a
    // synthetic click: the view moves and nothing announces it.
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *tree = browser(*window);
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("data.bin"));
    QVERIFY(file);
    auto *hex = window->findChild<QPlainTextEdit *>(QStringLiteral("hex"));

    const int line = static_cast<int>(file->data(0, tree::RoleBlock).toUInt()) * 512 /
                     hexview::BytesPerLine;
    {
        const QSignalBlocker silence(hex->verticalScrollBar());
        hex->verticalScrollBar()->setValue(line);
    }
    QVERIFY2(!markedRow(*window), "nothing announced the scroll, so nothing has run yet");

    QTRY_COMPARE_WITH_TIMEOUT(markedRow(*window), file, 2000);
    QVERIFY(status(*window).contains(QStringLiteral("data.bin")));
}

void TestMainWindow::theFollowStopsWhenAFileIsShown() {
    // The poll exists for the whole-disk view and must not outlive it: a file's
    // own bytes are one file, and a timer still walking the tree over a
    // document that has nothing to do with the disk map is waste at best.
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *tree = browser(*window);
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("data.bin"));
    QVERIFY(file);
    tree->setCurrentItem(file, 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();

    auto *hex = window->findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    const QString before = status(*window);
    {
        const QSignalBlocker silence(hex->verticalScrollBar());
        hex->verticalScrollBar()->setValue(hex->verticalScrollBar()->maximum());
    }
    QTest::qWait(400);
    QCOMPARE(status(*window), before);
    QVERIFY(!markedRow(*window));
}

void TestMainWindow::thereIsAHelpMenuWithAnAboutBox() {
    MainWindow window;
    QMenu *help = nullptr;
    for (QAction *top : window.menuBar()->actions()) {
        if (top->text().contains(QStringLiteral("Help"))) help = top->menu();
    }
    QVERIFY2(help, "the menu bar has a Help menu");

    QStringList items;
    for (QAction *item : help->actions()) items << item->text();
    QCOMPARE(items.size(), 1);
    QVERIFY2(items.first().contains(QStringLiteral("About")), qPrintable(items.first()));

    // Deliberately only About. The manual will join it; nothing stands in for
    // it in the meantime, because a menu item that is greyed out or opens an
    // apology is a promise the window has already broken.
}

void TestMainWindow::theAboutBoxTakesItsVersionFromTheEngine() {
    // Not from a string written in Qt, which can disagree with the library it
    // was built against — and the About box is the one place people go to find
    // out exactly what they are running.
    const QString engine = QString::fromUtf8(ade_version());
    QVERIFY(!engine.isEmpty());
    QVERIFY2(MainWindow::aboutTitle().contains(engine), qPrintable(MainWindow::aboutTitle()));

    // And the licence claim, which NOTICE also makes: if D-009 ever brings
    // xDMS in, this line has to change with it.
    QVERIFY(MainWindow::aboutDetail().contains(QStringLiteral("Apache License 2.0")));
    QVERIFY(MainWindow::aboutDetail().contains(QStringLiteral("no third-party code")));
}

// Content search in the window (F-021 through the ABI).
namespace {

// Run a search in the given mode and return the results tree.
ImageTree *runSearch(MainWindow &window, int mode, const QString &query) {
    auto *box = window.findChild<QComboBox *>(QStringLiteral("mode"));
    auto *field = window.findChild<QLineEdit *>();
    box->setCurrentIndex(mode);
    field->setText(query);
    emit field->returnPressed();
    QApplication::processEvents();
    return results(window);
}

}  // namespace

void TestMainWindow::theSearchBoxCanSearchContentsInsteadOfNames() {
    MainWindow window;
    window.resize(1200, 700);
    window.show();
    window.openImage(m_image);

    // Names first, which is what the box has always done.
    ImageTree *found = runSearch(window, 0, QStringLiteral("startup"));
    QVERIFY(found->topLevelItemCount() > 0);
    QCOMPARE(found->headerItem()->text(0), QStringLiteral("Name"));

    // Then contents: the same box, a different question, and columns that suit
    // the answer — an offset and what part of the disk it landed in.
    found = runSearch(window, 1, QStringLiteral("DOS"));
    QCOMPARE(found->headerItem()->text(0), QStringLiteral("Offset"));
    QVERIFY2(found->topLevelItemCount() > 0, "every AmigaDOS disk says DOS in its bootblock");

    // The first hit is block 0, and it is named as the bootblock rather than
    // as unallocated space.
    QCOMPARE(found->topLevelItem(0)->text(0), QStringLiteral("0"));
    QVERIFY2(found->topLevelItem(0)->text(1).contains(QStringLiteral("bootblock")),
             qPrintable(found->topLevelItem(0)->text(1)));

    // Contents reaches what names cannot: the bootblock is in no file.
    QVERIFY(status(window).contains(QStringLiteral("match")));
}

void TestMainWindow::aRefusedPatternSaysWhyRatherThanFindingNothing() {
    // "The pattern was refused" and "the pattern is not on this disk" are
    // different answers. Reporting the first as `0 matches` would have someone
    // conclude their disk is clean when nothing was ever searched — the same
    // distinction the command line draws with exit 2 against exit 1.
    MainWindow window;
    window.resize(1200, 700);
    window.show();
    window.openImage(m_image);

    ImageTree *found = runSearch(window, 1, QStringLiteral("0x601"));
    QCOMPARE(found->topLevelItemCount(), 0);
    QVERIFY2(status(window).contains(QStringLiteral("Cannot search")), qPrintable(status(window)));
    QVERIFY2(status(window).contains(QStringLiteral("hex digits")), qPrintable(status(window)));

    // Against a pattern that is fine and simply is not there.
    found = runSearch(window, 1, QStringLiteral("zzzznotonthisdisk"));
    QCOMPARE(found->topLevelItemCount(), 0);
    QVERIFY2(status(window).contains(QStringLiteral("0 matches")), qPrintable(status(window)));
    QVERIFY(!status(window).contains(QStringLiteral("Cannot search")));
}

void TestMainWindow::clickingAContentHitGoesToItInTheWholeDiskView() {
    // A list of offsets you cannot go to is half a feature.
    MainWindow window;
    window.resize(1200, 700);
    window.show();
    window.openImage(m_image);
    ImageTree *found = runSearch(window, 1, QStringLiteral("hello from a generated fixture"));
    QVERIFY2(found->topLevelItemCount() > 0, "the fixture's own file contents");

    const quint64 offset = found->topLevelItem(0)->text(0).toULongLong();
    QVERIFY(offset > 0);
    found->setCurrentItem(found->topLevelItem(0), 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();

    auto *hex = window.findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    QVERIFY(hex);
    // The whole disk, not the file: the hit is an offset into the image.
    QCOMPARE(hex->document()->blockCount() - 1, 901120 / hexview::BytesPerLine);

    // Scrolled so the hit is on screen, with a few lines of lead — a match on
    // the very first visible line reads as the top of the view rather than as
    // a result.
    const int hitLine = static_cast<int>(offset / hexview::BytesPerLine);
    const int top = hex->verticalScrollBar()->value();
    QVERIFY2(top <= hitLine && hitLine - top <= 6,
             qPrintable(QStringLiteral("hit on line %1, view at %2").arg(hitLine).arg(top)));

    // And the hit itself is highlighted, so it can be picked out of the line.
    QVERIFY(!hex->extraSelections().isEmpty());
}

// Extract everything to a folder (F-024).
//
// What the extraction *does* is tested in the engine (`src/api/tests/unpack.rs`)
// and across the ABI (`bridge/tests/abi.rs`), where the names, the skipping and
// the never-overwriting all live. The window's own share is the menu item and
// when it is offered — its action opens a folder chooser, which cannot be
// driven headlessly, and reaching past it into the window to re-test the
// engine would be duplicate coverage bought with a leaky accessor.
void TestMainWindow::extractingEverythingIsOfferedOnlyWithADiskToExtract() {
    MainWindow window;
    QAction *all = nullptr;
    for (QAction *top : window.menuBar()->actions()) {
        if (!top->text().contains(QStringLiteral("File"))) continue;
        for (QAction *item : top->menu()->actions()) {
            if (item->text().contains(QStringLiteral("all files"))) all = item;
        }
    }
    QVERIFY2(all, "File holds an Extract all files... item");
    QVERIFY2(!all->isEnabled(), "with no disk open there is nothing to extract");

    window.openImage(m_image);
    auto *tree = browser(window);
    QVERIFY(tree && tree->topLevelItemCount() > 0);
    tree->setCurrentItem(tree->topLevelItem(0), 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();
    QVERIFY2(all->isEnabled(), "with a disk selected it is offered");

    // And a file row still offers it: extracting everything takes the disk the
    // row belongs to, not the row.
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("startup"));
    QVERIFY(file);
    tree->setCurrentItem(file, 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();
    QVERIFY(all->isEnabled());

    window.closeAll();
    QApplication::processEvents();
    QVERIFY2(!all->isEnabled(), "and closing the disks takes it away again");
}

// Making a disk from the window (F-019, F-025).
//
// The action opens a modal dialog and then a file chooser, neither of which
// can be driven headlessly, and what happens after them is tested across the
// ABI (`bridge/tests/abi.rs`) and in C. The window's own share is offering the
// item at all and not inventing a list of filesystems.
void TestMainWindow::newDiskIsOfferedAndItsTypesComeFromTheEngine() {
    MainWindow window;
    QAction *neu = nullptr;
    for (QAction *top : window.menuBar()->actions()) {
        if (!top->text().contains(QStringLiteral("File"))) continue;
        for (QAction *item : top->menu()->actions()) {
            if (item->text().contains(QStringLiteral("New disk"))) neu = item;
        }
    }
    QVERIFY2(neu, "File holds a New disk... item");
    QVERIFY2(neu->isEnabled(), "and it needs no disk open to be useful");
    QCOMPARE(neu->shortcut(), QKeySequence(QKeySequence::New));

    // The window must not hold its own list of filesystems: two front ends
    // deciding separately which disks exist is two chances to disagree with
    // the engine. Six, because D-013 defers LNFS.
    QCOMPARE(ade_create_type_count(), size_t(6));
    for (size_t i = 0; i < ade_create_type_count(); ++i) {
        QVERIFY(*ade_create_type_name(i) != '\0');
        QVERIFY(*ade_create_type_label(i) != '\0');
    }
}

void TestMainWindow::aFileThatIsNotADiskImageIsDeclinedRatherThanShown() {
    // BUG-010, reported from the window: dragging three files out of a disk
    // and dropping them back opened two Amiga executables and a level file as
    // damaged hard disks. Three rows, each explaining nothing.
    //
    // Declining is not the same as declining everything unmountable. A DMS
    // archive is recognised and unreadable, and opening it to say so is the
    // point (IMP-006); an executable is neither.
    MainWindow window;
    QStringList errors;
    QObject::connect(&window, &MainWindow::errorOccurred, &window,
                     [&errors](const QString &message) { errors << message; });

    // The Amiga hunk magic and nothing else, which is what those files were.
    QByteArray executable(5732, '\0');
    executable[2] = '\x03';
    executable[3] = '\xF3';
    const QString exe = m_dir.filePath(QStringLiteral("program"));
    QFile out(exe);
    QVERIFY(out.open(QIODevice::WriteOnly));
    out.write(executable);
    out.close();

    window.openImage(exe);
    auto *tree = browser(window);
    QVERIFY(tree);
    QCOMPARE(tree->topLevelItemCount(), 0);
    QCOMPARE(errors.size(), 1);
    QVERIFY2(errors.first().contains(QStringLiteral("not a disk image")),
             qPrintable(errors.first()));

    // And a real image still opens, so the check has not become a refusal to
    // open anything.
    window.openImage(m_image);
    QCOMPARE(tree->topLevelItemCount(), 1);
    QCOMPARE(errors.size(), 1);

    // An unformatted floppy-sized file is recognised by its size and opens: it
    // is a disk image that holds no volume, which is a thing worth showing.
    const QString blank = m_dir.filePath(QStringLiteral("blank.adf"));
    QFile empty(blank);
    QVERIFY(empty.open(QIODevice::WriteOnly));
    empty.write(QByteArray(901120, '\0'));
    empty.close();
    window.openImage(blank);
    QCOMPARE(tree->topLevelItemCount(), 2);
    QCOMPARE(errors.size(), 1);
}

// The block map (F-027): a picture of where the space went.
namespace {

/// Contrast ratio, so "visible" is measured rather than asserted by eye.
double contrast(const QColor &a, const QColor &b) {
    const auto channel = [](double c) {
        return c <= 0.03928 ? c / 12.92 : std::pow((c + 0.055) / 1.055, 2.4);
    };
    const auto luminance = [&](const QColor &c) {
        return 0.2126 * channel(c.redF()) + 0.7152 * channel(c.greenF()) +
               0.0722 * channel(c.blueF());
    };
    const double x = luminance(a);
    const double y = luminance(b);
    return (std::max(x, y) + 0.05) / (std::min(x, y) + 0.05);
}

}  // namespace

void TestMainWindow::theMapColoursFilesAndKeepsEmptySpaceVisible() {
    // Two properties, both deliberate and both easy to get wrong.
    for (const auto &[label, page] : {std::pair{"dark", QColor(36, 36, 36)},
                                      std::pair{"light", QColor(255, 255, 255)}}) {
        QPalette palette;
        palette.setColor(QPalette::Base, page);

        // Files are coloured *here*, unlike in the hex pane. There the tint
        // sits under text being read, so colouring four fifths of a disk would
        // distinguish nothing; here every cell is a block and leaving files
        // blank would leave the map empty.
        const QColor file = MapView::colourFor(palette, ADE_REGION_FILE);
        QVERIFY2(file.isValid(), label);
        QVERIFY2(!HexHighlighter::regionColour(palette, ADE_REGION_FILE).isValid(),
                 "and the hex pane still leaves them alone");

        // Empty space must recede without disappearing. At the first values it
        // sat at 1.16 against the page, which made "empty" and "off the end of
        // the disk" the same picture.
        const QColor empty = MapView::colourFor(palette, ADE_REGION_UNCLAIMED);
        QVERIFY2(contrast(empty, page) > 1.3,
                 qPrintable(QStringLiteral("%1: unclaimed is invisible at %2")
                                .arg(label)
                                .arg(contrast(empty, page))));
        QVERIFY2(contrast(file, page) > 1.8, label);
        QVERIFY2(contrast(file, empty) > 1.45,
                 qPrintable(QStringLiteral("%1: file and empty look alike at %2")
                                .arg(label)
                                .arg(contrast(file, empty))));

        // And every structural region is distinct from every other.
        QList<QColor> seen;
        for (int region : {ADE_REGION_BOOTBLOCK, ADE_REGION_ROOTBLOCK, ADE_REGION_BITMAP,
                           ADE_REGION_DIRECTORY, ADE_REGION_FILE, ADE_REGION_UNCLAIMED}) {
            const QColor c = MapView::colourFor(palette, region);
            for (const QColor &other : seen) QVERIFY2(c != other, label);
            seen << c;
        }
    }
}

void TestMainWindow::theMapShowsTheDiskAndClickingACellGoesToIt() {
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *map = window->findChild<MapView *>(QStringLiteral("map"));
    QVERIFY2(map, "there is a Map tab");
    auto *legend = window->findChild<QLabel *>(QStringLiteral("mapLegend"));
    QVERIFY(legend);
    QVERIFY2(legend->text().contains(QStringLiteral("bootblock")), qPrintable(legend->text()));

    // Clicking the first cell is asking what is in block 0, and the answer is
    // the bytes: the whole-disk view, scrolled there.
    map->resize(400, 400);
    QMouseEvent press(QEvent::MouseButtonPress, QPointF(1, 1), map->mapToGlobal(QPointF(1, 1)),
                      Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QApplication::sendEvent(map, &press);
    QApplication::processEvents();

    auto *hex = window->findChild<QPlainTextEdit *>(QStringLiteral("hex"));
    QVERIFY(hex);
    QCOMPARE(hex->verticalScrollBar()->value(), 0);
    QVERIFY2(status(*window).contains(QStringLiteral("bootblock")), qPrintable(status(*window)));
    // And it brought the hex view to the front, rather than scrolling a tab
    // nobody is looking at.
    auto *tabs = window->findChild<QTabWidget *>();
    QVERIFY(tabs);
    QCOMPARE(tabs->tabText(tabs->currentIndex()), QStringLiteral("Hex"));
}

void TestMainWindow::selectingAFilePicksOutItsBlocksOnTheMap() {
    // The thing a listing cannot show: where a file actually lives. The map
    // belongs to the disk, so selecting a file must not clear it — it picks
    // that file's blocks out instead.
    QScopedPointer<MainWindow> window(diskWindow(m_image));
    auto *map = window->findChild<MapView *>(QStringLiteral("map"));
    QVERIFY(map);
    map->resize(400, 400);

    const QImage before = map->grab().toImage();

    auto *tree = browser(*window);
    tree->expandItem(tree->topLevelItem(0));
    QTreeWidgetItem *file = childNamed(tree->topLevelItem(0), QStringLiteral("data.bin"));
    QVERIFY(file);
    tree->setCurrentItem(file, 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();

    const QImage after = map->grab().toImage();
    QVERIFY2(before != after, "the file's blocks are picked out");

    // Not by clearing the map: most of it is unchanged, because most of the
    // disk is not that file.
    int changed = 0;
    for (int y = 0; y < qMin(before.height(), after.height()); y += 3) {
        for (int x = 0; x < qMin(before.width(), after.width()); x += 3) {
            if (before.pixel(x, y) != after.pixel(x, y)) ++changed;
        }
    }
    const int sampled = (before.height() / 3) * (before.width() / 3);
    QVERIFY2(changed > 0 && changed < sampled / 2,
             qPrintable(QStringLiteral("%1 of %2 sampled pixels changed").arg(changed).arg(sampled)));
}

void TestMainWindow::thereIsADiskMenuOfferedOnlyWithADiskOpen() {
    // What a disk *is* and what it *needs* are the window's two standing
    // questions about the thing on screen, so they get a menu rather than
    // living under File. The dialog itself is modal and cannot be driven
    // headlessly; what it says is tested in the engine and across the ABI.
    MainWindow window;
    QMenu *menu = nullptr;
    for (QAction *top : window.menuBar()->actions()) {
        if (top->text().contains(QStringLiteral("Disk"))) menu = top->menu();
    }
    QVERIFY2(menu, "the menu bar has a Disk menu");
    QVERIFY(!menu->actions().isEmpty());
    QAction *info = menu->actions().first();
    QVERIFY2(info->text().contains(QStringLiteral("information")), qPrintable(info->text()));
    QVERIFY2(!info->isEnabled(), "with no disk open there is nothing to describe");

    window.openImage(m_image);
    auto *tree = browser(window);
    tree->setCurrentItem(tree->topLevelItem(0), 0, QItemSelectionModel::ClearAndSelect);
    QApplication::processEvents();
    QVERIFY(info->isEnabled());

    // And the engine has something to say about that disk, with evidence.
    QVERIFY(ade_specs_unknowable_count() >= 4);
}

QTEST_MAIN(TestMainWindow)
#include "test_mainwindow.moc"
