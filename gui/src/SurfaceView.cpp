#include "SurfaceView.h"

#include <QHelpEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QToolTip>

namespace {

constexpr int Cylinders = 80;
constexpr int Heads = 2;
/// Room for the "head 0" labels down the left.
constexpr int LabelWidth = 58;
constexpr int RulerHeight = 16;

}  // namespace

SurfaceView::SurfaceView(QWidget *parent) : QWidget(parent) {
    setMinimumSize(420, 120);
    setMouseTracking(true);
}

void SurfaceView::setTracks(const QVector<AdeTrack> &tracks) {
    m_tracks = tracks;
    update();
}

QColor SurfaceView::colourFor(const QPalette &palette, const AdeTrack &track) {
    const QColor base = palette.color(QPalette::Base);
    const auto wash = [&base](int r, int g, int b, int percent) {
        const auto mix = [&](int from, int to) { return from + (to - from) * percent / 100; };
        return QColor(mix(r, base.red()), mix(g, base.green()), mix(b, base.blue()));
    };
    if (track.source == ADE_TRACK_ABSENT) {
        // Not in the container at all. Recessive, because it is the absence of
        // a measurement rather than a bad one — but not invisible: the same
        // mistake the block map made first time, where empty space sat at 1.16
        // contrast against the page and "empty" and "off the disk" became the
        // same picture.
        return wash(120, 120, 130, 70);
    }
    if (track.sectors == 0) return wash(200, 60, 60, 15);   // read, gave nothing
    if (track.sectors >= track.expected) return wash(60, 160, 90, 25);  // whole
    return wash(210, 160, 40, 20);                          // partial
}

QString SurfaceView::legend(const QPalette &palette) {
    const auto swatch = [&palette](const QColor &colour, const QString &label) {
        return QStringLiteral("<span style='background:%1; border:1px solid %2'>"
                              "&nbsp;&nbsp;&nbsp;</span>&nbsp;%3")
            .arg(colour.name(), palette.color(QPalette::Mid).name(), label);
    };
    AdeTrack whole{};
    whole.sectors = 11;
    whole.expected = 11;
    whole.source = ADE_TRACK_RAW_MFM;
    AdeTrack partial = whole;
    partial.sectors = 5;
    AdeTrack nothing = whole;
    nothing.sectors = 0;
    AdeTrack absent = whole;
    absent.source = ADE_TRACK_ABSENT;

    return QStringLiteral("%1&nbsp;&nbsp;&nbsp;&nbsp;%2&nbsp;&nbsp;&nbsp;&nbsp;%3"
                          "&nbsp;&nbsp;&nbsp;&nbsp;%4")
        .arg(swatch(colourFor(palette, whole), QStringLiteral("whole track")),
             swatch(colourFor(palette, partial), QStringLiteral("some sectors")),
             swatch(colourFor(palette, nothing), QStringLiteral("read, nothing decoded")),
             swatch(colourFor(palette, absent), QStringLiteral("not in the container")));
}

QRect SurfaceView::cellFor(int cylinder, int head) const {
    const int usable = width() - LabelWidth - 4;
    if (usable <= 0) return {};
    const int cell = qMax(2, usable / Cylinders);
    // The rows use the height they are given rather than a fixed 40, which
    // left two thin strips at the top of a dialog and a lot of nothing under
    // them. Capped only so a very tall window does not make two enormous bars.
    const int rowHeight = qMax(10, qMin(90, (height() - RulerHeight - 8) / Heads));
    return QRect(LabelWidth + cylinder * cell, RulerHeight + head * rowHeight, cell - 1,
                 rowHeight - 2);
}

void SurfaceView::paintEvent(QPaintEvent *event) {
    QPainter painter(this);
    painter.fillRect(event->rect(), palette().color(QPalette::Base));
    if (m_tracks.isEmpty()) return;

    painter.setPen(palette().color(QPalette::Text));
    const QFont small = font();
    painter.setFont(small);

    // A ruler every ten cylinders, which is how somebody counts to a track.
    for (int cylinder = 0; cylinder < Cylinders; cylinder += 10) {
        const QRect cell = cellFor(cylinder, 0);
        if (cell.isEmpty()) continue;
        painter.drawText(cell.left(), RulerHeight - 4, QString::number(cylinder));
    }

    for (int head = 0; head < Heads; ++head) {
        const QRect first = cellFor(0, head);
        if (!first.isEmpty()) {
            painter.drawText(2, first.bottom() - 2, QStringLiteral("head %1").arg(head));
        }
        for (int cylinder = 0; cylinder < Cylinders; ++cylinder) {
            const int index = cylinder * Heads + head;
            if (index >= m_tracks.size()) continue;
            const QRect cell = cellFor(cylinder, head);
            if (cell.isEmpty()) continue;
            painter.fillRect(cell, colourFor(palette(), m_tracks[index]));
        }
    }
}

int SurfaceView::trackAt(const QPoint &where) const {
    for (int head = 0; head < Heads; ++head) {
        for (int cylinder = 0; cylinder < Cylinders; ++cylinder) {
            if (cellFor(cylinder, head).adjusted(0, 0, 1, 2).contains(where)) {
                return cylinder * Heads + head;
            }
        }
    }
    return -1;
}

bool SurfaceView::event(QEvent *event) {
    if (event->type() == QEvent::ToolTip) {
        auto *help = static_cast<QHelpEvent *>(event);
        const int index = trackAt(help->pos());
        if (index >= 0 && index < m_tracks.size()) {
            const AdeTrack &t = m_tracks[index];
            const QString source = t.source == ADE_TRACK_ABSENT
                                       ? QStringLiteral("not in the container")
                                       : (t.source == ADE_TRACK_SECTORS
                                              ? QStringLiteral("stored as sectors")
                                              : QStringLiteral("decoded from raw MFM"));
            QToolTip::showText(help->globalPos(),
                               QStringLiteral("cylinder %1, head %2 — %3 of %4 sectors — %5")
                                   .arg(t.cylinder)
                                   .arg(t.head)
                                   .arg(t.sectors)
                                   .arg(t.expected)
                                   .arg(source),
                               this);
        } else {
            QToolTip::hideText();
        }
        return true;
    }
    return QWidget::event(event);
}
