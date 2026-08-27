#include "ImageTree.h"

#include <QDir>
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
    if (!m_extractor || items.isEmpty() || !dragDir().isValid()) return nullptr;

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
    if (urls.isEmpty()) return nullptr;

    auto *mime = new QMimeData;
    mime->setUrls(urls);
    return mime;
}
