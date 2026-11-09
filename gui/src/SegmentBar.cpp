#include "SegmentBar.h"

#include <QHelpEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QToolTip>

SegmentBar::SegmentBar(QWidget *parent) : QWidget(parent) {
    setFixedWidth(16);
    setMouseTracking(true);
    setToolTip(QString());
}

void SegmentBar::setRuns(const QVector<Run> &runs) {
    m_runs = runs;
    m_total = 0;
    for (const Run &r : m_runs) m_total += r.length;
    update();
}

void SegmentBar::clear() {
    m_runs.clear();
    m_total = 0;
    update();
}

QColor SegmentBar::colourFor(const QPalette &palette, AdeSegmentClass kind) {
    const QColor base = palette.color(QPalette::Base);
    const auto wash = [&base](int r, int g, int b, int percent) {
        const auto mix = [&](int from, int to) { return from + (to - from) * percent / 100; };
        return QColor(mix(r, base.red()), mix(g, base.green()), mix(b, base.blue()));
    };
    switch (kind) {
        case ADE_SEG_REACHED:   return wash(70, 130, 220, 15);   // code: the strongest
        case ADE_SEG_UNREACHED: return wash(70, 130, 220, 65);   // code none reached: faded
        case ADE_SEG_TEXT:      return wash(220, 170, 40, 20);   // strings
        case ADE_SEG_DATA:      return wash(60, 170, 90, 35);    // data
        default:                return wash(120, 120, 130, 80);  // zeroed: recessive
    }
}

QString SegmentBar::nameOf(AdeSegmentClass kind) {
    switch (kind) {
        case ADE_SEG_REACHED:   return QStringLiteral("code the walks reached");
        case ADE_SEG_UNREACHED: return QStringLiteral("code hunk bytes no walk reached");
        case ADE_SEG_TEXT:      return QStringLiteral("a string");
        case ADE_SEG_DATA:      return QStringLiteral("data");
        default:                return QStringLiteral("zeroed memory");
    }
}

int SegmentBar::runAt(int y, quint64 *into) const {
    if (m_total == 0 || height() <= 0) return -1;
    const quint64 at = static_cast<quint64>(qBound(0, y, height() - 1)) * m_total /
                       static_cast<quint64>(height());
    quint64 seen = 0;
    for (int i = 0; i < m_runs.size(); ++i) {
        const quint64 len = m_runs[i].length;
        if (at < seen + len) {
            if (into) *into = at - seen;
            return i;
        }
        seen += len;
    }
    return -1;
}

void SegmentBar::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), palette().color(QPalette::Base));
    if (m_total == 0) return;
    const double scale = static_cast<double>(height()) / static_cast<double>(m_total);
    quint64 seen = 0;
    quint32 hunk = m_runs.isEmpty() ? 0 : m_runs.first().hunk;
    const QColor rule = palette().color(QPalette::Text);
    for (const Run &r : m_runs) {
        const int top = static_cast<int>(static_cast<double>(seen) * scale);
        seen += r.length;
        // At least a pixel: a short string or a lone reached routine is
        // exactly what the bar exists to show.
        const int bottom = qMax(top + 1, static_cast<int>(static_cast<double>(seen) * scale));
        painter.fillRect(QRect(0, top, width(), bottom - top), colourFor(palette(), r.kind));
        if (r.hunk != hunk) {
            painter.fillRect(QRect(0, top, width(), 1), rule);
            hunk = r.hunk;
        }
    }
}

void SegmentBar::mousePressEvent(QMouseEvent *event) {
    quint64 into = 0;
    const int i = runAt(static_cast<int>(event->position().y()), &into);
    if (i < 0 || !m_runs[i].hasOffset) return;
    emit jumpTo(m_runs[i].offset + into);
}

bool SegmentBar::event(QEvent *event) {
    if (event->type() == QEvent::ToolTip) {
        auto *help = static_cast<QHelpEvent *>(event);
        const int i = runAt(help->pos().y(), nullptr);
        if (i < 0) {
            QToolTip::hideText();
        } else {
            const Run &r = m_runs[i];
            QToolTip::showText(help->globalPos(),
                               QStringLiteral("hunk %1, $%2–$%3: %4")
                                   .arg(r.hunk)
                                   .arg(r.start, 0, 16)
                                   .arg(r.start + r.length, 0, 16)
                                   .arg(nameOf(r.kind)),
                               this);
        }
        return true;
    }
    return QWidget::event(event);
}
