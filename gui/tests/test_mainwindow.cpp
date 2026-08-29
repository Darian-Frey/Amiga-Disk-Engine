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
    HexNullDimmer dimmer(pane.document(), &pane);

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
    const QColor dimLight = HexNullDimmer::dimColour(light);
    QVERIFY(dimLight.lightness() > QColor(Qt::black).lightness());
    QVERIFY(dimLight.lightness() < QColor(Qt::white).lightness());

    QPalette dark;
    dark.setColor(QPalette::Text, Qt::white);
    dark.setColor(QPalette::Base, QColor(30, 30, 30));
    const QColor dimDark = HexNullDimmer::dimColour(dark);
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

QTEST_MAIN(TestMainWindow)
#include "test_mainwindow.moc"
