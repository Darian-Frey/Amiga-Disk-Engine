// A tree that can drag files out of a disk image (F-004).
//
// Dragging to a file manager means offering `text/uri-list`, which means the
// file has to exist somewhere first. So the bytes are extracted to a temporary
// file at the moment the drag starts — not before, because most selections are
// never dragged anywhere.

#pragma once

#include <QTreeWidget>

#include <QDropEvent>
#include <QUrl>

#include <functional>

class ImageTree : public QTreeWidget {
    Q_OBJECT

public:
    using Extractor = std::function<QByteArray(QTreeWidgetItem *)>;

    explicit ImageTree(QWidget *parent = nullptr);

    // How to get a file's bytes. Set by the window, which owns the images —
    // the tree deliberately knows nothing about them.
    void setExtractor(Extractor extractor) { m_extractor = std::move(extractor); }

    // Public, unlike the base class's, so a test can ask what a drag would
    // carry without having to stage a real one through the window system.
    QMimeData *mimeData(const QList<QTreeWidgetItem *> &items) const override;

    // Accept files dropped *onto* the tree, and say which row they landed on.
    // Off by default: the search results take no drops.
    void setAcceptsFiles(bool accept);

    // Whether a drag from `source` carrying `mime` would be taken: files from
    // somewhere other than this tree, or rows dragged within it. A drag that
    // began here is never taken as files — let go a pixel from where it
    // started, it would copy itself back in — but as rows, which the window
    // may move. Public because a test cannot give a drop a source without
    // starting a real drag through the window system.
    bool wouldAccept(const QObject *source, const QMimeData *mime) const;

    // The format a drag that began in this tree carries, so a drop can tell
    // rows being moved from files being copied.
    static QString rowsFormat() { return QStringLiteral("application/x-ade-rows"); }

signals:
    // Host files dropped onto `target`, or onto empty space when it is null.
    // The window decides what that means — copy them onto a disk, or open
    // them as disks — because the tree knows nothing about images.
    void filesDropped(QTreeWidgetItem *target, const QList<QUrl> &urls);
    // Rows of this tree dragged onto `target`. The window decides what that
    // means, which within one disk is a move.
    void rowsDropped(QTreeWidgetItem *target, const QList<QTreeWidgetItem *> &rows);

protected:
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dragMoveEvent(QDragMoveEvent *event) override;
    void dropEvent(QDropEvent *event) override;

private:
    bool wants(const QDropEvent *event) const {
        return wouldAccept(event->source(), event->mimeData());
    }

    bool m_acceptsFiles = false;
    Extractor m_extractor;
};
