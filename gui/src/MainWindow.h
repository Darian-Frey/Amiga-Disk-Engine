// The main window (F-004): a directory tree, a hex view, a preview, and
// search across every image that is open.
#pragma once

#include "Image.h"

#include <QMainWindow>

#include <memory>
#include <utility>
#include <vector>

class ImageTree;
class QAction;
class QLabel;
class QComboBox;
class QLineEdit;
class QTimer;
/// How the tree stores what each row stands for.
///
/// In the header rather than beside the window's implementation because the
/// tests read them: a test that checks the right row was marked has to ask the
/// row which block it is, and a second copy of `Qt::UserRole + 1` in the test
/// is a copy that can drift.
namespace tree {
/// Columns in the tree.
enum Column { ColName = 0, ColSize, ColDate, ColProtection };

// Which block an item stands for, and whether it is a directory.
constexpr int RoleBlock = Qt::UserRole + 1;
constexpr int RoleIsDir = Qt::UserRole + 2;
constexpr int RolePopulated = Qt::UserRole + 3;
// Which open image an item belongs to. Every item carries it, so a click in
// the tree or in the search results knows which disk it means.
constexpr int RoleImage = Qt::UserRole + 4;
// Which partition of that image, or ADE_WHOLE_IMAGE for one that holds its own
// volume. Carried by every item for the same reason: a block number means
// nothing without the volume it belongs to, and on a hard disk the same number
// is a different block in every partition.
constexpr int RolePartition = Qt::UserRole + 5;
/// Where in the image a content-search hit was found. Only search-result rows
/// carry it; its absence is what tells a click that the row is a file rather
/// than an offset.
constexpr int RoleOffset = Qt::UserRole + 6;
}  // namespace tree

class HexHighlighter;
class HexPane;
class QPlainTextEdit;
class QTabWidget;
class QTreeWidgetItem;

class MainWindow : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(QWidget *parent = nullptr);

    // Open an image, adding it to whatever is already loaded. Failure is
    // reported through `errorOccurred` rather than shown here.
    void openImage(const QString &path);

    /// The About box's heading and its detail, as rich text.
    ///
    /// Public and static so a test can read what the box would say without
    /// opening it: `QMessageBox::exec` blocks, and a test that has to dismiss a
    /// modal dialog to check its contents is a test that hangs the suite the
    /// day the dialog changes.
    static QString aboutTitle();
    static QString aboutDetail();

private slots:
    void newDisk();
    void extractAll();
    void showAbout();
    void updateSearchHint();
    void searchContents(const QString &query);

public:

    // Close every open image.
    void closeAll();

    // How many images are open. Several can be, which is what makes
    // cross-image search meaningful.
    size_t imageCount() const { return m_images.size(); }

signals:
    // Something went wrong that a person should see.
    //
    // The window does not raise its own dialogs. A modal box blocks until it
    // is clicked, which makes every failure path untestable and, worse, means
    // the window decides how errors are surfaced for every future front end.
    // `main.cpp` connects this to a QMessageBox; the tests connect it to a
    // list, and so can check that failures are reported at all.
    void errorOccurred(const QString &message);

protected:
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dropEvent(QDropEvent *event) override;

private slots:
    void chooseImage();
    void extractSelected();
    void search();

private:
    // One open image and where it came from.
    struct Open {
        ade::Image image;
        QString path;
        QString name;
        size_t index = 0;
    };

    void addImageRoot(Open &open);
    // One row per partition of a device, each with its files beneath it.
    void addPartition(QTreeWidgetItem *root, const Open &open, quint32 index,
                      const AdePartition &partition);
    void populate(QTreeWidgetItem *parent, const Open &open, quint32 partition, quint32 block);
    // Every mountable volume of an image, as (partition selector, label). One
    // entry for a floppy, one per mounting partition for a hard disk.
    static std::vector<std::pair<quint32, QString>> volumesOf(const Open &open);
    // Show an entry in the hex and text views, from either tree.
    void showEntry(QTreeWidgetItem *item);
    const Open *imageFor(const QTreeWidgetItem *item) const;
    // A file entry's bytes, or empty for a directory or an unreadable one.
    void showHit(QTreeWidgetItem *item);
    void showWholeDisk(QTreeWidgetItem *item);
    void showLegend(const QVector<HexRegion> &regions);
    void markWhatIsOnScreen();
    void markRow(quint32 block);
    QByteArray contentsOf(QTreeWidgetItem *item) const;
    // One line describing an image: container, volume, size, findings.
    static QString describe(const Open &open);
    void showSummary();
    void clearViews();

    // The dataset, loaded once at startup when one is configured. Images
    // opened afterwards are named as they open (F-013).
    ade::Catalogue m_catalogue;
    std::vector<std::unique_ptr<Open>> m_images;
    // Set while one tree is clearing the other's selection, so that the
    // clearing does not come straight back as a selection change.
    bool m_syncing = false;
    // The entry last shown, from whichever tree. Extraction acts on it, so
    // that a search result extracts as readily as a tree row.
    QTreeWidgetItem *m_selected = nullptr;

    ImageTree *m_tree = nullptr;
    ImageTree *m_results = nullptr;
    HexPane *m_hex = nullptr;
    HexHighlighter *m_paint = nullptr;
    QComboBox *m_mode = nullptr;
    /// The Hex tab's page, so a search hit can bring it to the front.
    QWidget *m_hexTab = nullptr;
    QLabel *m_legend = nullptr;
    /// The open disk's map, while the whole-disk view is showing it.
    QVector<HexRegion> m_diskRegions;
    /// Polls the scroll position while a whole disk is shown.
    QTimer *m_follow = nullptr;
    /// The last top line acted on, so a tick that changed nothing costs one
    /// comparison.
    int m_topLine = -1;
    /// Which image and partition the whole-disk view is showing, so a row is
    /// looked up in the disk being scrolled rather than in whichever open
    /// image happens to hold that block number.
    qulonglong m_diskImage = 0;
    quint32 m_diskPartition = 0;
    /// The row currently marked as "this is what you are looking at".
    QTreeWidgetItem *m_marked = nullptr;
    QPlainTextEdit *m_text = nullptr;
    QLineEdit *m_query = nullptr;
    QTabWidget *m_views = nullptr;
    QLabel *m_summary = nullptr;
    QAction *m_extract = nullptr;
    QAction *m_extractAll = nullptr;
};
