#include "MapView.h"

#include <QHelpEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QToolTip>
#include <algorithm>

namespace {

/// Region codes, in the order a cell that covers several should prefer.
///
/// A cell on a large disk stands for many blocks, and showing the *most
/// structural* of them keeps a single rootblock visible on a map where it is a
/// fiftieth of a pixel. Showing the commonest instead would erase exactly the
/// blocks worth finding.
int rank(int region) {
    switch (region) {
        case 0:  return 5;  // bootblock
        case 1:  return 4;  // rootblock
        case 2:  return 3;  // bitmap
        case 3:  return 2;  // directory
        case 4:  return 1;  // file
        default: return 0;  // unclaimed
    }
}

/// The smallest cell worth drawing. Below this the grid is a texture rather
/// than a map, and aggregating blocks per cell is the better trade.
constexpr int MinCell = 3;
constexpr int MaxCell = 14;

}  // namespace

MapView::MapView(QWidget *parent) : QWidget(parent) {
    setMinimumSize(120, 120);
    setMouseTracking(true);
}

QColor MapView::colourFor(const QPalette &palette, int region) {
    const QColor base = palette.color(QPalette::Base);
    const auto wash = [&base](int r, int g, int b, int percent) {
        const auto mix = [&](int from, int to) { return from + (to - from) * percent / 100; };
        return QColor(mix(r, base.red()), mix(g, base.green()), mix(b, base.blue()));
    };
    switch (region) {
        case 0:  return wash(220, 60, 60, 25);    // bootblock
        case 1:  return wash(220, 170, 40, 25);   // rootblock
        case 2:  return wash(60, 170, 90, 25);    // bitmap
        case 3:  return wash(70, 130, 220, 25);   // directory
        // File and unclaimed carry most of the map between them, so their
        // percentages were solved rather than chosen. At the first values
        // unclaimed sat at 1.16 contrast against the page — invisible, which
        // made "empty" and "off the end of the disk" the same picture. These
        // hold up in both themes: on dark, file 3.93 and unclaimed 1.39
        // against the page with 2.82 between them; on light, 2.27 and 1.41
        // with 1.61 between.
        case 4:  return wash(150, 150, 165, 20);  // file — the bulk
        default: return wash(120, 120, 130, 72);  // unclaimed — recessive, not absent
    }
}

void MapView::setMap(const QVector<HexRegion> &spans, quint64 blocks, quint32 blockSize) {
    m_blockSize = blockSize > 0 ? blockSize : 512;
    m_highlight = 0;
    const int count = static_cast<int>(qMin<quint64>(blocks, 4'000'000));
    m_region.fill(5, count);  // unclaimed until a span says otherwise
    m_owner.fill(0, count);
    m_paths.fill(QString(), count);

    for (const HexRegion &span : spans) {
        const quint64 first = span.start / m_blockSize;
        const quint64 last = span.end / m_blockSize;
        for (quint64 b = first; b < last && b < static_cast<quint64>(count); ++b) {
            const int at = static_cast<int>(b);
            m_region[at] = static_cast<quint8>(span.region);
            m_owner[at] = span.owner;
            m_paths[at] = span.path;
        }
    }
    update();
}

void MapView::clear() {
    m_region.clear();
    m_owner.clear();
    m_paths.clear();
    m_highlight = 0;
    update();
}

void MapView::highlightOwner(quint32 ownerBlock) {
    if (m_highlight == ownerBlock) return;
    m_highlight = ownerBlock;
    update();
}

MapView::Grid MapView::grid() const {
    Grid g;
    if (m_region.isEmpty() || width() <= 0 || height() <= 0) return g;
    const quint64 blocks = static_cast<quint64>(m_region.size());

    // The largest cell at which every block still fits. Big disks fall through
    // to the smallest and then aggregate.
    for (int cell = MaxCell; cell >= MinCell; --cell) {
        const quint64 columns = static_cast<quint64>(width() / cell);
        const quint64 rows = static_cast<quint64>(height() / cell);
        if (columns == 0 || rows == 0) continue;
        if (columns * rows >= blocks) {
            g.cell = cell;
            g.columns = static_cast<int>(columns);
            g.rows = static_cast<int>((blocks + columns - 1) / columns);
            g.blocksPerCell = 1;
            return g;
        }
    }

    g.cell = MinCell;
    g.columns = qMax(1, width() / MinCell);
    const quint64 capacity =
        static_cast<quint64>(g.columns) * static_cast<quint64>(qMax(1, height() / MinCell));
    g.blocksPerCell = qMax<quint64>(1, (blocks + capacity - 1) / capacity);
    const quint64 cells = (blocks + g.blocksPerCell - 1) / g.blocksPerCell;
    g.rows = static_cast<int>((cells + static_cast<quint64>(g.columns) - 1) / g.columns);
    return g;
}

void MapView::paintEvent(QPaintEvent *event) {
    QPainter painter(this);
    painter.fillRect(event->rect(), palette().color(QPalette::Base));
    const Grid g = grid();
    if (g.cell == 0) return;

    const QColor highlight = palette().color(QPalette::Highlight);
    const quint64 blocks = static_cast<quint64>(m_region.size());

    for (int row = 0; row < g.rows; ++row) {
        for (int column = 0; column < g.columns; ++column) {
            const quint64 index = (static_cast<quint64>(row) * g.columns + column);
            const quint64 first = index * g.blocksPerCell;
            if (first >= blocks) break;

            // The most structural region this cell covers, and whether any of
            // it belongs to the highlighted entry.
            int best = m_region[static_cast<int>(first)];
            bool owned = false;
            const quint64 last = qMin(first + g.blocksPerCell, blocks);
            for (quint64 b = first; b < last; ++b) {
                const int at = static_cast<int>(b);
                if (rank(m_region[at]) > rank(best)) best = m_region[at];
                if (m_highlight != 0 && m_owner[at] == m_highlight) owned = true;
            }

            const QRect cell(column * g.cell, row * g.cell, g.cell - 1, g.cell - 1);
            painter.fillRect(cell, owned ? highlight : colourFor(palette(), best));
        }
    }
}

quint64 MapView::blockAt(const QPoint &where) const {
    const Grid g = grid();
    const quint64 blocks = static_cast<quint64>(m_region.size());
    if (g.cell == 0) return blocks;
    const int column = where.x() / g.cell;
    const int row = where.y() / g.cell;
    if (column < 0 || column >= g.columns || row < 0 || row >= g.rows) return blocks;
    const quint64 index = static_cast<quint64>(row) * g.columns + column;
    const quint64 block = index * g.blocksPerCell;
    return block < blocks ? block : blocks;
}

QString MapView::describe(quint64 block) const {
    const int at = static_cast<int>(block);
    if (at < 0 || at >= m_region.size()) return {};
    const QString region =
        QString::fromUtf8(ade_region_name(static_cast<AdeRegion>(m_region[at])));
    const Grid g = grid();

    QString where = g.blocksPerCell > 1
                        // Said, not hidden: a cell that stands for sixty-four
                        // blocks and reports one of them would be lying about
                        // its own resolution.
                        ? QStringLiteral("blocks %1–%2")
                              .arg(block)
                              .arg(qMin(block + g.blocksPerCell - 1,
                                        static_cast<quint64>(m_region.size() - 1)))
                        : QStringLiteral("block %1").arg(block);
    where += QStringLiteral("  ·  offset %1  ·  %2")
                 .arg(block * m_blockSize)
                 .arg(region);
    if (!m_paths[at].isEmpty()) where += QStringLiteral("  ·  %1").arg(m_paths[at]);
    return where;
}

bool MapView::event(QEvent *event) {
    if (event->type() == QEvent::ToolTip) {
        auto *help = static_cast<QHelpEvent *>(event);
        const quint64 block = blockAt(help->pos());
        if (block < static_cast<quint64>(m_region.size())) {
            QToolTip::showText(help->globalPos(), describe(block), this);
        } else {
            QToolTip::hideText();
        }
        return true;
    }
    return QWidget::event(event);
}

void MapView::mousePressEvent(QMouseEvent *event) {
    const quint64 block = blockAt(event->pos());
    if (block < static_cast<quint64>(m_region.size())) {
        emit blockChosen(block);
    }
}
