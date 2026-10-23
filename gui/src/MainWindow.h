// The main window (F-004): a directory tree, a hex view, a preview, and
// search across every image that is open.
#pragma once

#include "Image.h"

#include <QMainWindow>

#include <functional>
#include <memory>
#include <utility>
#include <vector>

class ImageTree;
class QAction;
class QLabel;
class QComboBox;
class QLineEdit;
class QPushButton;
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
class MapView;
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
    void showSurface();
    void recoverLostFiles();
    void showSpecs();
    void newDisk();
    void extractAll();
    void showAbout();
    void updateSearchHint();
    void searchContents(const QString &query);

public:

    // Close every open image. Asks first if any has unsaved edits, and returns
    // false — closing nothing — if the answer is no.
    bool closeAll();

    // --- Writing into a disk (D-018) --------------------------------------
    //
    // Public, and free of dialogs, so the tests can drive them: the menu
    // actions and the drop handler only ask for names and paths, then call
    // these.

    /// Copy host files and folders onto the disk `target` belongs to: into
    /// `target` itself when it is a directory or the disk's own row, beside it
    /// when it is a file. Folders are copied with everything in them. Returns
    /// how many entries were made; anything refused is reported through
    /// `errorOccurred`, all of it in one message.
    int addFiles(QTreeWidgetItem *target, const QStringList &paths);
    /// Make a directory, placed as `addFiles` places things.
    bool makeFolder(QTreeWidgetItem *target, const QString &name);
    /// Save the disk `item` belongs to as a new file. Never the file it was
    /// opened from; never over another existing file.
    bool saveImageAs(QTreeWidgetItem *item, const QString &path);
    /// Save: to the last Save As destination if there is one; otherwise over
    /// the file the disk was opened from, after asking (D-020), keeping the
    /// original beside it the first time. A disk this window created saves
    /// back to its own file without asking. Returns whether it saved.
    bool saveImage(QTreeWidgetItem *item = nullptr);
    /// Open a disk this window has just created (New disk), so its later
    /// saves go back to its own file.
    void openCreated(const QString &path);
    /// Undo or redo the last edit to the disk the actions act on.
    bool undoEdit();
    bool redoEdit();

    /// How the window asks whether unsaved edits may be thrown away.
    /// Defaults to a message box; a test replaces it, because a modal dialog
    /// blocks until somebody clicks it.
    using Confirm = std::function<bool(const QString &question)>;
    void setDiscardConfirmation(Confirm confirm) { m_confirmDiscard = std::move(confirm); }
    /// How the window asks whether to save over the disk that was opened.
    void setSaveConfirmation(Confirm confirm) { m_confirmSave = std::move(confirm); }

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
    void closeEvent(QCloseEvent *event) override;
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dropEvent(QDropEvent *event) override;

private slots:
    void chooseImage();
    void save();
    void saveAs();
    void newFolder();
    void filesDropped(QTreeWidgetItem *target, const QList<QUrl> &urls);
    void extractSelected();
    void search();

private:
    // One open image and where it came from.
    struct Open {
        ade::Image image;
        QString path;
        QString name;
        size_t index = 0;
        /// Made by this window's New disk: its file is ADE's own, so Save
        /// writes back to it without asking and without a backup (D-020).
        bool createdHere = false;
        /// Where the original was kept the first time Save wrote over it.
        QString backup;
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
    Open *imageFor(const QTreeWidgetItem *item);
    // The disk Save, Undo and the rest act on: the selection's, else the one
    // last edited, else the only one open.
    Open *editTarget();
    // The directory block `item` stands for, as a place to put things: itself
    // if a directory, its parent if a file, the root for a disk's own row.
    // Zero when it is not on a disk ADE can write into.
    quint32 directoryFor(const QTreeWidgetItem *item, const Open &open) const;
    // One host path onto the disk, folders recursively, counting what was
    // made and collecting what was refused.
    void addPath(Open &open, quint32 dir, const QString &path, int depth, int *made,
                 QStringList *refused);
    // Rebuild a disk's rows after it changed, keeping open what was open.
    void reloadImage(Open &open);
    void updateEditActions();
    // Whether unsaved edits may be thrown away: true when there are none.
    bool mayDiscardEdits();
    // A file entry's bytes, or empty for a directory or an unreadable one.
    void showHit(QTreeWidgetItem *item);
    void showWholeDisk(QTreeWidgetItem *item);
    void showLegend(const QVector<HexRegion> &regions);
    void showMapLegend(const QVector<HexRegion> &regions);
    void goToOffset(quint64 offset);
    void markWhatIsOnScreen();
    // Fill the Assembly view for the selection, if it is showing and stale.
    void refreshAssembly();
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
    MapView *m_map = nullptr;
    QLabel *m_mapLegend = nullptr;
    QWidget *m_mapTab = nullptr;
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
    QAction *m_carve = nullptr;
    QAction *m_specs = nullptr;
    QAction *m_surface = nullptr;
    /// The Assembly view (F-032).
    QWidget *m_asmTab = nullptr;
    QPlainTextEdit *m_asm = nullptr;
    QLabel *m_asmStatus = nullptr;
    QPushButton *m_asmRaw = nullptr;
    /// Whether the view no longer matches the selection.
    bool m_asmStale = true;
    /// Whether the selected file is to be decoded even though it is not a
    /// program. Reset whenever the selection changes.
    bool m_asmForceRaw = false;
    QAction *m_save = nullptr;
    QAction *m_saveAs = nullptr;
    QAction *m_undo = nullptr;
    QAction *m_redo = nullptr;
    QAction *m_newFolder = nullptr;
    /// The disk most recently edited, which Save and Undo fall back to when
    /// nothing is selected.
    size_t m_lastEdited = static_cast<size_t>(-1);
    Confirm m_confirmDiscard;
    Confirm m_confirmSave;
};
