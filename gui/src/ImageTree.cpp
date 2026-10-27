#include "ImageTree.h"

#include <QDir>
#include <QDragEnterEvent>
#include <QDragMoveEvent>
#include <QDropEvent>
#include <QFile>
#include <QMimeData>
#include <QTemporaryDir>
#include <QUrl>

namespace {

// One directory per session, cleaned up when the application exits. Writing
// each drag into the same place keeps the temporary files together and means a
// drag that is cancelled leaves one stale file rather than one per attempt.
QTemporaryDir &dragDir() {
    static QTemporaryDir dir;
    return dir;
}

}  // namespace

ImageTree::ImageTree(QWidget *parent) : QTreeWidget(parent) {
    setDragEnabled(true);
    setDragDropMode(QAbstractItemView::DragOnly);
    setSelectionMode(QAbstractItemView::ExtendedSelection);
}

QMimeData *ImageTree::mimeData(const QList<QTreeWidgetItem *> &items) const {
    if (items.isEmpty()) return nullptr;
    auto *mime = new QMimeData;
    // Always, so folders can be dragged within the tree: they carry no file.
    mime->setData(rowsFormat(), QByteArray("1"));
    if (!m_extractor || !dragDir().isValid()) return mime;

    QList<QUrl> urls;
    for (QTreeWidgetItem *item : items) {
        const QByteArray data = m_extractor(item);
        // An empty result means "not a readable file" — a directory, or a
        // damaged entry. Skipping it beats dragging out a zero-byte lie.
        if (data.isEmpty()) continue;

        const QString target = QDir(dragDir().path()).filePath(item->text(0));
        QFile out(target);
        if (!out.open(QIODevice::WriteOnly)) continue;
        out.write(data);
        out.close();
        urls << QUrl::fromLocalFile(target);
    }
    if (!urls.isEmpty()) mime->setUrls(urls);
    return mime;
}

void ImageTree::setAcceptsFiles(bool accept) {
    m_acceptsFiles = accept;
    setAcceptDrops(accept);
    viewport()->setAcceptDrops(accept);
    setDragDropMode(accept ? QAbstractItemView::DragDrop : QAbstractItemView::DragOnly);
    setDropIndicatorShown(accept);
}

bool ImageTree::wouldAccept(const QObject *source, const QMimeData *mime) const {
    if (!m_acceptsFiles || !mime) return false;
    if (source == this) return mime->hasFormat(rowsFormat());
    return mime->hasUrls();
}

void ImageTree::dragEnterEvent(QDragEnterEvent *event) {
    if (!wants(event)) {
        event->ignore();
        return;
    }
    event->setDropAction(Qt::CopyAction);
    event->accept();
}

void ImageTree::dragMoveEvent(QDragMoveEvent *event) {
    if (!wants(event)) {
        event->ignore();
        return;
    }
    event->setDropAction(Qt::CopyAction);
    event->accept();
}

void ImageTree::dropEvent(QDropEvent *event) {
    if (!wants(event)) {
        event->ignore();
        return;
    }
    // Copy, never move: a move would ask the source to delete its file —
    // and from within this tree, would have Qt delete the dragged rows
    // itself, under the window that is about to rebuild them.
    event->setDropAction(Qt::CopyAction);
    event->accept();
    QTreeWidgetItem *target = itemAt(event->position().toPoint());
    if (event->source() == this) {
        emit rowsDropped(target, selectedItems());
        return;
    }
    emit filesDropped(target, event->mimeData()->urls());
}
