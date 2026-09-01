#pragma once

// A picture of what occupies a disk (F-027), over the map F-022 already builds.
//
// The hex pane answers "what is at this offset" one screen at a time. A grid of
// blocks answers "where did the space go" at a glance, which is the question a
// health report gives numbers for and a dump never gets to.

#include "HexView.h"

#include <ade.h>

#include <QVector>
#include <QWidget>

class MapView : public QWidget {
    Q_OBJECT

public:
    explicit MapView(QWidget *parent = nullptr);

    /// Show `spans` covering `blocks` blocks of `blockSize` bytes.
    ///
    /// The spans must tile with no gaps, which is what `ade_layout_open`
    /// guarantees — a hole would be a cell painted in no colour at all, which
    /// reads as free space rather than as missing information.
    void setMap(const QVector<HexRegion> &spans, quint64 blocks, quint32 blockSize);
    void clear();

    /// Pick out the blocks belonging to one directory entry, or none for 0.
    ///
    /// Selecting a file in the tree and seeing where it actually lives is the
    /// thing this view can do that no listing can: a file's blocks are
    /// scattered wherever the bitmap had room, and "fragmented" stops being an
    /// abstraction the moment it is drawn.
    void highlightOwner(quint32 ownerBlock);

    /// The colour a region is drawn in.
    ///
    /// **Files are coloured here, unlike in the hex pane.** There the tint sits
    /// under text being read, so colouring the four fifths of a disk that hold
    /// file data would colour everything and distinguish nothing. Here every
    /// cell *is* a block and the whole question is where the space went, so
    /// leaving files blank would leave the map empty.
    static QColor colourFor(const QPalette &palette, int region);

signals:
    /// A block was clicked.
    void blockChosen(quint64 block);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    bool event(QEvent *event) override;

private:
    /// Where a cell sits, and what it covers.
    struct Grid {
        int cell = 0;            ///< pixels per side
        int columns = 0;
        int rows = 0;
        quint64 blocksPerCell = 1;  ///< above 1 on a disk with more blocks than cells
    };

    Grid grid() const;
    /// The block a viewport point falls on, or `blocks()` past the end.
    quint64 blockAt(const QPoint &where) const;
    /// What to say about a block.
    QString describe(quint64 block) const;

    /// One region code per block, and the owning entry per block.
    QVector<quint8> m_region;
    QVector<quint32> m_owner;
    QVector<QString> m_paths;   ///< owner path per block, empty where none
    quint32 m_blockSize = 512;
    quint32 m_highlight = 0;
};
