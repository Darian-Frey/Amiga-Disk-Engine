#include "HexView.h"

#include <QApplication>
#include <QClipboard>
#include <QContextMenuEvent>
#include <QGlyphRun>
#include <QKeyEvent>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QRawFont>
#include <QScrollBar>

#include <algorithm>
#include <climits>
#include <cmath>

namespace hexview {

QString line(quint64 offset, const QByteArray &bytes) {
    QString out;
    out.reserve(LineLength);
    out += QStringLiteral("%1  ").arg(offset, 8, 16, QChar('0'));
    QString ascii;
    for (int i = 0; i < BytesPerLine; ++i) {
        if (i < bytes.size()) {
            const unsigned char c = static_cast<unsigned char>(bytes[i]);
            out += QStringLiteral("%1 ").arg(c, 2, 16, QChar('0'));
            ascii += (c >= 0x20 && c < 0x7F) ? QChar(c) : QChar('.');
        } else {
            // Three spaces, not "00": a line that runs out of data has no
            // byte there at all, and showing one would be an invention.
            out += QStringLiteral("   ");
        }
        if (i == BytesPerLine / 2 - 1) out += QChar(' ');
    }
    out += QStringLiteral(" |") + ascii + QStringLiteral("|");
    return out;
}

}  // namespace hexview

namespace {

using hexview::AsciiColumn;
using hexview::BytesPerLine;
using hexview::columnOf;
using hexview::FieldColumn;
using hexview::Zone;

/// Whether `line` is a dump line at all.
///
/// The pane also shows "(no readable contents)", and a selection dragged over
/// that must not try to index columns it does not have.
bool isDumpLine(const QString &line) { return line.size() >= AsciiColumn; }

/// The byte `column` names, clamped into the field.
int byteIndexAt(Zone zone, int column) {
    if (zone == Zone::Ascii) {
        return std::clamp(column - AsciiColumn, 0, BytesPerLine - 1);
    }
    // The hex field's columns are not evenly spaced — a gap splits the two
    // groups of eight — so the byte is found rather than divided out.
    for (int i = BytesPerLine - 1; i > 0; --i) {
        if (column >= columnOf(i)) return i;
    }
    return 0;
}

/// The character range in a line covering bytes `from` through `to`.
QPair<int, int> spanOf(Zone zone, int from, int to) {
    switch (zone) {
        case Zone::Offset:
            return {0, 8};
        case Zone::Hex:
            return {columnOf(from), columnOf(to) + 2 - columnOf(from)};
        case Zone::Ascii:
            return {AsciiColumn + from, to - from + 1};
    }
    return {0, 0};
}

/// `percent` of the way from `from` to `to`.
QColor blend(const QColor &from, const QColor &to, int percent) {
    const auto mix = [percent](int a, int b) { return a + (b - a) * percent / 100; };
    return QColor(mix(from.red(), to.red()), mix(from.green(), to.green()),
                  mix(from.blue(), to.blue()));
}

}  // namespace

HexPane::HexPane(QWidget *parent) : QAbstractScrollArea(parent) {
    viewport()->setBackgroundRole(QPalette::Base);
    viewport()->setAutoFillBackground(true);
    viewport()->setCursor(Qt::IBeamCursor);
    setFocusPolicy(Qt::StrongFocus);
    m_autoScroll.setInterval(50);
    connect(&m_autoScroll, &QTimer::timeout, this, [this] { dragTo(m_lastDrag); });
    updateScrollBars();
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

void HexPane::setBytes(const QByteArray &bytes) {
    clear();
    m_bytes = bytes;
    m_size = static_cast<quint64>(bytes.size());
    updateScrollBars();
    viewport()->update();
}

void HexPane::setSource(quint64 size, Reader read) {
    clear();
    m_read = std::move(read);
    m_size = std::min(size, MaxBytes);
    updateScrollBars();
    viewport()->update();
}

void HexPane::setMessage(const QString &message) {
    clear();
    m_message = message;
    updateScrollBars();
    viewport()->update();
}

void HexPane::clear() {
    m_bytes.clear();
    m_read = nullptr;
    m_size = 0;
    m_message.clear();
    m_regions.clear();
    m_hasSelection = false;
    m_selecting = false;
    m_autoScroll.stop();
    verticalScrollBar()->setValue(0);
    horizontalScrollBar()->setValue(0);
    updateScrollBars();
    viewport()->update();
}

void HexPane::setRegions(QVector<HexRegion> regions) {
    m_regions = std::move(regions);
    viewport()->update();
}

quint64 HexPane::size() const { return m_size; }

int HexPane::lineCount() const {
    if (!m_message.isEmpty()) return 1;
    const quint64 lines = (m_size + BytesPerLine - 1) / BytesPerLine;
    return static_cast<int>(std::min<quint64>(lines, INT_MAX));
}

QByteArray HexPane::bytesOf(int first, int last) const {
    const quint64 from = static_cast<quint64>(first) * BytesPerLine;
    if (from >= m_size || last < first) return {};
    const quint64 to = std::min(m_size, (static_cast<quint64>(last) + 1) * BytesPerLine);
    if (m_read) return m_read(from, to - from);
    return m_bytes.mid(static_cast<qsizetype>(from), static_cast<qsizetype>(to - from));
}

QString HexPane::lineText(int number) const {
    if (number < 0 || number >= lineCount()) return {};
    if (!m_message.isEmpty()) return m_message;
    return hexview::line(static_cast<quint64>(number) * BytesPerLine, bytesOf(number, number));
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

QColor HexPane::regionColour(const QPalette &palette, int region) {
    // Hues rather than palette colours, because six things have to be told
    // apart and a palette offers two. Each is then mixed most of the way into
    // the page, so the same six read correctly on a light theme and a dark one
    // and the text stays the text rather than becoming a label on a swatch.
    const QColor base = palette.color(QPalette::Base);
    const auto wash = [&base](int r, int g, int b, int percent) {
        return blend(QColor(r, g, b), base, percent);
    };
    switch (region) {
        case 0:  return wash(220, 60, 60, 78);    // bootblock — where protection lives
        case 1:  return wash(220, 170, 40, 78);   // rootblock
        case 2:  return wash(60, 170, 90, 78);    // bitmap
        case 3:  return wash(70, 130, 220, 78);   // directory
        case 4:  return QColor();                 // file — most of the disk; left alone
        case 5:  return wash(140, 140, 150, 90);  // unclaimed — the faintest wash there is
        default: return QColor();
    }
}

QColor HexPane::dimColour(const QPalette &palette) {
    return blend(palette.color(QPalette::Text), palette.color(QPalette::Base), 70);
}

QColor HexPane::mirrorColour(const QPalette &palette) {
    return blend(palette.color(QPalette::Highlight), palette.color(QPalette::Base), 66);
}

int HexPane::regionAt(quint64 offset) const {
    // Binary search: a hard disk's map has as many spans as it has files, and
    // this runs for every visible line on every repaint.
    int low = 0;
    int high = m_regions.size() - 1;
    while (low <= high) {
        const int mid = low + (high - low) / 2;
        const HexRegion &span = m_regions.at(mid);
        if (offset < span.start) {
            high = mid - 1;
        } else if (offset >= span.end) {
            low = mid + 1;
        } else {
            return span.region;
        }
    }
    return -1;
}

// ---------------------------------------------------------------------------
// What a line looks like
// ---------------------------------------------------------------------------

std::pair<HexPane::Point, HexPane::Point> HexPane::ordered() const {
    Point first = m_anchor;
    Point last = m_cursor;
    if (last.line < first.line || (last.line == first.line && last.index < first.index)) {
        std::swap(first, last);
    }
    return {first, last};
}

QVector<HexPane::Mark> HexPane::marksOn(int number) const {
    return marksFor(number, lineText(number));
}

QVector<HexPane::Mark> HexPane::marksFor(int number, const QString &text) const {
    QVector<Mark> out;
    if (!isDumpLine(text)) return out;
    const QPalette &p = palette();

    // The region tint first, under everything: it is the background, and the
    // dimming that follows only ever sets a foreground. A line is sixteen
    // bytes and a region boundary is a block boundary, so a line falls inside
    // exactly one region — 512 is a whole number of lines.
    if (!m_regions.isEmpty()) {
        const QColor tint =
            regionColour(p, regionAt(static_cast<quint64>(number) * BytesPerLine));
        if (tint.isValid()) out.append({Mark::Tint, 0, static_cast<int>(text.size()), tint, {}});
    }

    const QColor dim = dimColour(p);
    for (int i = 0; i < BytesPerLine; ++i) {
        const int column = columnOf(i);
        if (column + 2 > text.size()) break;
        if (text[column] == QLatin1Char('0') && text[column + 1] == QLatin1Char('0')) {
            out.append({Mark::Dim, column, 2, {}, dim});
        }
    }

    if (!m_hasSelection) return out;
    const auto [first, last] = ordered();
    if (number < first.line || number > last.line) return out;
    const int from = (number == first.line) ? first.index : 0;
    const int to = (number == last.line) ? last.index : BytesPerLine - 1;

    // Selecting offsets is selecting whole lines, and their hex and characters
    // are then both "corresponding" — marking every column of every line says
    // nothing the highlighted offsets have not already said. The mirror goes
    // first, so an overlap could only ever be painted over by the selection.
    // No foreground on it: the text must stay in its ordinary colour, dimmed
    // nulls included, or the mark reads as a second selection.
    if (m_zone != Zone::Offset) {
        const auto [start, length] = spanOf(m_zone == Zone::Hex ? Zone::Ascii : Zone::Hex, from, to);
        out.append({Mark::Mirror, start, std::min(length, static_cast<int>(text.size()) - start),
                    mirrorColour(p), {}});
    }
    const auto [start, length] = spanOf(m_zone, from, to);
    out.append({Mark::Selection, start, std::min(length, static_cast<int>(text.size()) - start),
                p.color(QPalette::Highlight), p.color(QPalette::HighlightedText)});
    return out;
}

// ---------------------------------------------------------------------------
// Geometry and painting
// ---------------------------------------------------------------------------

qreal HexPane::charWidth() const {
    // Measured over a whole line, not one character: a line drawn in one run
    // advances by the font's hinted width, which one character's advance does
    // not show — measured, a 2 px drift by the end of a line.
    const QString sample(hexview::LineLength, QLatin1Char('0'));
    return std::max<qreal>(1, QFontMetricsF(font()).horizontalAdvance(sample) / sample.size());
}

int HexPane::lineHeight() const { return std::max(1, fontMetrics().lineSpacing()); }

void HexPane::updateScrollBars() {
    const QSize area = viewport()->size();
    const int visible = std::max(1, (area.height() - 2 * Margin) / lineHeight());
    QScrollBar *v = verticalScrollBar();
    v->setRange(0, std::max(0, lineCount() - visible));
    v->setPageStep(visible);
    v->setSingleStep(1);

    const int longest = m_message.isEmpty() ? hexview::LineLength : static_cast<int>(m_message.size());
    const int width = static_cast<int>(std::ceil(longest * charWidth())) + 2 * Margin;
    QScrollBar *h = horizontalScrollBar();
    h->setRange(0, std::max(0, width - area.width()));
    h->setPageStep(area.width());
    h->setSingleStep(static_cast<int>(std::ceil(charWidth())));
}

QPoint HexPane::pointOf(int number, int column) const {
    const int y = Margin + (number - verticalScrollBar()->value()) * lineHeight() + lineHeight() / 2;
    const int x =
        static_cast<int>(Margin + column * charWidth()) - horizontalScrollBar()->value() + 1;
    return {x, y};
}

bool HexPane::pointAt(const QPoint &pos, bool clamp, Zone &zone, Point &point) const {
    const int count = lineCount();
    if (count == 0) return false;
    int number = verticalScrollBar()->value() +
                 static_cast<int>(std::floor(double(pos.y() - Margin) / lineHeight()));
    if (number < 0 || number >= count) {
        if (!clamp) return false;
        number = std::clamp(number, 0, count - 1);
    }
    const QString text = lineText(number);
    if (!isDumpLine(text)) return false;
    // The nearest character boundary, as a text cursor would land.
    const double x = double(pos.x() + horizontalScrollBar()->value() - Margin) / charWidth();
    const int column = std::clamp(static_cast<int>(std::lround(x)), 0, static_cast<int>(text.size()));
    zone = hexview::zoneAt(column);
    point.line = number;
    point.index = byteIndexAt(zone, column);
    return true;
}

void HexPane::paintEvent(QPaintEvent *) {
    QPainter painter(viewport());
    painter.setFont(font());
    const QPalette &p = palette();
    const qreal width = charWidth();
    const int height = lineHeight();
    // Every glyph is placed on its column explicitly. Drawn as strings, a
    // run's text advances by however the font rounds, and a line split into
    // many colours ends up a pixel or two from a line drawn in one — so the
    // columns of two lines would not line up. Glyph lookup by character also
    // means no ligatures: `ff` is two cells, as everything here assumes.
    const QRawFont raw = QRawFont::fromFont(font());
    const int ascent = fontMetrics().ascent();
    const qreal left = Margin - horizontalScrollBar()->value();

    const int top = verticalScrollBar()->value();
    const int rows = viewport()->height() / height + 2;
    const int last = std::min(lineCount() - 1, top + rows);
    if (last < top) return;

    // One read for the screen, not one per line: a whole disk is read through
    // the engine, and forty calls would be forty trips across the C ABI.
    const QByteArray screen = m_message.isEmpty() ? bytesOf(top, last) : QByteArray();

    for (int number = top; number <= last; ++number) {
        QString text;
        if (!m_message.isEmpty()) {
            text = m_message;
        } else {
            const qsizetype at = static_cast<qsizetype>(number - top) * BytesPerLine;
            text = hexview::line(static_cast<quint64>(number) * BytesPerLine,
                                 screen.mid(at, BytesPerLine));
        }
        const int y = Margin + (number - top) * height;

        // Each column's colours, then drawn in runs: a later mark wins.
        QVector<QColor> background(text.size());
        QVector<QColor> foreground(text.size(), p.color(QPalette::Text));
        for (const Mark &mark : marksFor(number, text)) {
            for (int c = mark.column; c < mark.column + mark.length && c < text.size(); ++c) {
                if (c < 0) continue;
                if (mark.background.isValid()) background[c] = mark.background;
                if (mark.foreground.isValid()) foreground[c] = mark.foreground;
            }
        }
        for (int c = 0; c < text.size();) {
            int end = c + 1;
            while (end < text.size() && background[end] == background[c]) ++end;
            if (background[c].isValid()) {
                painter.fillRect(QRectF(left + c * width, y, (end - c) * width, height),
                                 background[c]);
            }
            c = end;
        }
        const QVector<quint32> glyphs = raw.glyphIndexesForString(text);
        for (int c = 0; c < text.size() && c < glyphs.size();) {
            int end = c + 1;
            while (end < text.size() && foreground[end] == foreground[c]) ++end;
            QVector<QPointF> positions;
            positions.reserve(end - c);
            for (int i = c; i < end; ++i) positions.append(QPointF(i * width, 0));
            QGlyphRun run;
            run.setRawFont(raw);
            run.setGlyphIndexes(glyphs.mid(c, end - c));
            run.setPositions(positions);
            painter.setPen(foreground[c]);
            painter.drawGlyphRun(QPointF(left, y + ascent), run);
            c = end;
        }
    }
}

void HexPane::resizeEvent(QResizeEvent *event) {
    QAbstractScrollArea::resizeEvent(event);
    updateScrollBars();
}

void HexPane::changeEvent(QEvent *event) {
    QAbstractScrollArea::changeEvent(event);
    if (event->type() == QEvent::FontChange) updateScrollBars();
    if (event->type() == QEvent::FontChange || event->type() == QEvent::PaletteChange) {
        viewport()->update();
    }
}

void HexPane::scrollContentsBy(int, int) {
    // Everything on screen depends on the top line, so a scroll repaints
    // rather than blitting.
    viewport()->update();
    emit scrolled();
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

int HexPane::copyEnd(int first, int last) const {
    const int most = static_cast<int>(CopyLimit / BytesPerLine);
    return last - first >= most ? first + most - 1 : last;
}

quint64 HexPane::bytesBetween(int first, int last) const {
    const quint64 from = static_cast<quint64>(first) * BytesPerLine;
    const quint64 to = std::min(m_size, (static_cast<quint64>(last) + 1) * BytesPerLine);
    return to > from ? to - from : 0;
}

void HexPane::reportCut(int first, int last) {
    const int end = copyEnd(first, last);
    if (end != last) emit copyCut(bytesBetween(first, end), bytesBetween(first, last));
}

QString HexPane::selectedFieldText() const {
    if (!m_hasSelection) return {};
    auto [first, last] = ordered();
    if (copyEnd(first.line, last.line) != last.line) {
        // Cut: the last line taken is taken whole.
        last = {copyEnd(first.line, last.line), BytesPerLine - 1};
    }
    const QByteArray bytes = bytesOf(first.line, last.line);
    QStringList lines;
    for (int number = first.line; number <= last.line; ++number) {
        const qsizetype at = static_cast<qsizetype>(number - first.line) * BytesPerLine;
        const QString text =
            hexview::line(static_cast<quint64>(number) * BytesPerLine, bytes.mid(at, BytesPerLine));
        const int from = (number == first.line) ? first.index : 0;
        const int to = (number == last.line) ? last.index : BytesPerLine - 1;
        const auto [start, length] = spanOf(m_zone, from, to);
        // Trailing space only in the hex field, where every byte carries one.
        lines.append(text.mid(start, length).trimmed());
    }
    return lines.join(QChar('\n'));
}

QString HexPane::selectedLines() const {
    if (!m_hasSelection) return {};
    auto [first, last] = ordered();
    last.line = copyEnd(first.line, last.line);
    const QByteArray bytes = bytesOf(first.line, last.line);
    QStringList lines;
    for (int number = first.line; number <= last.line; ++number) {
        const qsizetype at = static_cast<qsizetype>(number - first.line) * BytesPerLine;
        lines.append(
            hexview::line(static_cast<quint64>(number) * BytesPerLine, bytes.mid(at, BytesPerLine)));
    }
    return lines.join(QChar('\n'));
}

void HexPane::copyLines() {
    const QString text = selectedLines();
    if (text.isEmpty()) return;
    QApplication::clipboard()->setText(text);
    reportCut(ordered().first.line, ordered().second.line);
}

void HexPane::copyField() {
    const QString text = selectedFieldText();
    if (text.isEmpty()) return;
    QApplication::clipboard()->setText(text);
    reportCut(ordered().first.line, ordered().second.line);
}

void HexPane::selectBytesAt(quint64 offset) {
    const quint64 line = offset / BytesPerLine;
    if (!m_message.isEmpty() || line >= static_cast<quint64>(lineCount())) return;
    m_zone = Zone::Hex;
    m_anchor = {static_cast<int>(line), static_cast<int>(offset % BytesPerLine)};
    m_cursor = m_anchor;
    m_hasSelection = true;
    viewport()->update();
}

void HexPane::selectField() {
    if (!m_message.isEmpty()) return;
    const int lines = lineCount();
    m_anchor = {0, 0};
    m_cursor = {lines - 1, BytesPerLine - 1};
    m_hasSelection = lines > 0;
    viewport()->update();
}

void HexPane::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) {
        QAbstractScrollArea::mousePressEvent(event);
        return;
    }
    Zone zone = Zone::Hex;
    Point point;
    // A press with no drag is not a selection yet, so a plain click clears
    // whatever was highlighted rather than leaving a one-byte remnant.
    m_hasSelection = false;
    viewport()->update();
    if (!pointAt(event->pos(), false, zone, point)) return;
    m_zone = zone;
    m_anchor = point;
    m_cursor = point;
    m_selecting = true;
}

void HexPane::dragTo(const QPoint &pos) {
    // Held past the top or bottom edge, the view scrolls toward the pointer
    // and the selection follows it — a line per tick, as a text view does.
    QScrollBar *v = verticalScrollBar();
    if (pos.y() < 0) {
        v->setValue(v->value() - 1);
    } else if (pos.y() > viewport()->height()) {
        v->setValue(v->value() + 1);
    }
    Zone zone = Zone::Hex;
    Point point;
    // A drag that strays into another field keeps going in the one it started
    // in — the alternative is a selection that changes meaning underneath the
    // pointer, which is the behaviour being fixed.
    if (pointAt(pos, true, zone, point)) {
        m_cursor = point;
        m_hasSelection = (m_cursor.line != m_anchor.line || m_cursor.index != m_anchor.index);
        viewport()->update();
    }
}

void HexPane::mouseMoveEvent(QMouseEvent *event) {
    if (!m_selecting) {
        QAbstractScrollArea::mouseMoveEvent(event);
        return;
    }
    m_lastDrag = event->pos();
    dragTo(m_lastDrag);
    const bool outside = m_lastDrag.y() < 0 || m_lastDrag.y() > viewport()->height();
    if (outside && !m_autoScroll.isActive()) m_autoScroll.start();
    if (!outside) m_autoScroll.stop();
}

void HexPane::mouseReleaseEvent(QMouseEvent *event) {
    m_selecting = false;
    m_autoScroll.stop();
    QAbstractScrollArea::mouseReleaseEvent(event);
}

void HexPane::mouseDoubleClickEvent(QMouseEvent *event) {
    Zone zone = Zone::Hex;
    Point point;
    if (!pointAt(event->pos(), false, zone, point)) {
        QAbstractScrollArea::mouseDoubleClickEvent(event);
        return;
    }
    // The whole field on that line, which is the unit a double click means
    // here: a "word" in a hex dump is one line of one column.
    m_zone = zone;
    m_anchor = {point.line, 0};
    m_cursor = {point.line, BytesPerLine - 1};
    m_hasSelection = true;
    m_selecting = false;
    viewport()->update();
}

void HexPane::keyPressEvent(QKeyEvent *event) {
    if (event->matches(QKeySequence::Copy)) {
        copyField();
        event->accept();
        return;
    }
    if (event->matches(QKeySequence::SelectAll)) {
        selectField();
        event->accept();
        return;
    }
    // The scroll area moves by lines and pages on its own; the two ends are
    // the keys it leaves out.
    if (event->matches(QKeySequence::MoveToStartOfDocument)) {
        verticalScrollBar()->setValue(0);
        event->accept();
        return;
    }
    if (event->matches(QKeySequence::MoveToEndOfDocument)) {
        verticalScrollBar()->setValue(verticalScrollBar()->maximum());
        event->accept();
        return;
    }
    QAbstractScrollArea::keyPressEvent(event);
}

void HexPane::contextMenuEvent(QContextMenuEvent *event) {
    QMenu menu(this);
    QAction *copy = menu.addAction(tr("&Copy"));
    copy->setShortcut(QKeySequence::Copy);
    copy->setEnabled(m_hasSelection);
    connect(copy, &QAction::triggered, this, &HexPane::copyField);

    QAction *lines = menu.addAction(tr("Copy Whole &Lines"));
    lines->setEnabled(m_hasSelection);
    connect(lines, &QAction::triggered, this, &HexPane::copyLines);

    menu.addSeparator();
    QAction *all = menu.addAction(tr("Select &All"));
    all->setShortcut(QKeySequence::SelectAll);
    connect(all, &QAction::triggered, this, &HexPane::selectField);

    menu.exec(event->globalPos());
}
