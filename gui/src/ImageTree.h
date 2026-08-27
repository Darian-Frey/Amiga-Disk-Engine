// A tree that can drag files out of a disk image (F-004).
//
// Dragging to a file manager means offering `text/uri-list`, which means the
// file has to exist somewhere first. So the bytes are extracted to a temporary
// file at the moment the drag starts — not before, because most selections are
// never dragged anywhere.

#pragma once

#include <QTreeWidget>

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

private:
    Extractor m_extractor;
};
