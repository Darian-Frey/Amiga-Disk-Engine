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
class QLineEdit;
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
    void showWholeDisk(QTreeWidgetItem *item);
    void showLegend(const QVector<HexRegion> &regions);
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
    QLabel *m_legend = nullptr;
    QPlainTextEdit *m_text = nullptr;
    QLineEdit *m_query = nullptr;
    QTabWidget *m_views = nullptr;
    QLabel *m_summary = nullptr;
    QAction *m_extract = nullptr;
};
