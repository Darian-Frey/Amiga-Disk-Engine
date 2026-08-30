#include "HexView.h"

#include <QApplication>
#include <QByteArray>
#include <QClipboard>
#include <QContextMenuEvent>
#include <QKeyEvent>
#include <QMenu>
#include <QMouseEvent>
#include <QScrollBar>
#include <QTextBlock>
#include <QTextCharFormat>

namespace hexview {

QString dump(const QByteArray &data) {
    QString out;
    out.reserve(data.size() * 4);
    for (int offset = 0; offset < data.size(); offset += BytesPerLine) {
        out += QStringLiteral("%1  ").arg(offset, 8, 16, QChar('0'));
        QString ascii;
        for (int i = 0; i < BytesPerLine; ++i) {
            if (offset + i < data.size()) {
                const unsigned char c = static_cast<unsigned char>(data[offset + i]);
                out += QStringLiteral("%1 ").arg(c, 2, 16, QChar('0'));
                ascii += (c >= 0x20 && c < 0x7F) ? QChar(c) : QChar('.');
            } else {
                // Three spaces, not "00": a line that runs out of data has no
                // byte there at all, and showing one would be an invention.
                out += QStringLiteral("   ");
            }
            if (i == BytesPerLine / 2 - 1) out += QChar(' ');
        }
        out += QStringLiteral(" |") + ascii + QStringLiteral("|\n");
    }
    return out;
}

}  // namespace hexview

HexHighlighter::HexHighlighter(QTextDocument *document, const QPlainTextEdit *source)
    : QSyntaxHighlighter(document), m_source(source) {}

void HexHighlighter::setRegions(QVector<HexRegion> regions) {
    m_regions = std::move(regions);
    rehighlight();
}

QColor HexHighlighter::regionColour(const QPalette &palette, int region) {
    // Hues rather than palette colours, because six things have to be told
    // apart and a palette offers two. Each is then mixed most of the way into
    // the page, so the same six read correctly on a light theme and a dark one
    // and the text stays the text rather than becoming a label on a swatch.
    const QColor base = palette.color(QPalette::Base);
    const auto wash = [&base](int r, int g, int b, int percent) {
        const auto mix = [&](int from, int to) { return from + (to - from) * percent / 100; };
        return QColor(mix(r, base.red()), mix(g, base.green()), mix(b, base.blue()));
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

int HexHighlighter::regionAt(quint64 offset) const {
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

QColor HexHighlighter::dimColour(const QPalette &palette) {
    const QColor text = palette.color(QPalette::Text);
    const QColor base = palette.color(QPalette::Base);
    constexpr int Toward = 70;  // percent of the way to the background
    const auto blend = [](int from, int to) { return from + (to - from) * Toward / 100; };
    return QColor(blend(text.red(), base.red()), blend(text.green(), base.green()),
                  blend(text.blue(), base.blue()));
}

void HexHighlighter::highlightBlock(const QString &line) {
    const QPalette palette = m_source ? m_source->palette() : QPalette();

    // The region tint first, under everything: it is the background, and the
    // dimming that follows only ever sets a foreground.
    //
    // A line is sixteen bytes and a region boundary is a block boundary, so a
    // line falls inside exactly one region — 512 is a whole number of lines.
    // The lookup is by the line's first byte for that reason.
    if (!m_regions.isEmpty()) {
        const quint64 offset =
            static_cast<quint64>(currentBlock().blockNumber()) * hexview::BytesPerLine;
        const QColor tint = regionColour(palette, regionAt(offset));
        if (tint.isValid()) {
            QTextCharFormat wash;
            wash.setBackground(tint);
            setFormat(0, line.size(), wash);
        }
    }

    QTextCharFormat dim;
    dim.setForeground(dimColour(palette));

    for (int i = 0; i < hexview::BytesPerLine; ++i) {
        const int column = hexview::columnOf(i);
        // A short final line, and any line that is not a dump line at all —
        // the pane also shows "(no readable contents)".
        if (column + 2 > line.size()) break;
        if (line[column] == QLatin1Char('0') && line[column + 1] == QLatin1Char('0')) {
            setFormat(column, 2, dim);
        }
    }
}

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

}  // namespace

HexPane::HexPane(QWidget *parent) : QPlainTextEdit(parent) {}

bool HexPane::pointAt(const QPoint &pos, Zone &zone, Point &point) const {
    const QTextCursor cursor = cursorForPosition(pos);
    const QString line = cursor.block().text();
    if (!isDumpLine(line)) return false;
    const int column = cursor.positionInBlock();
    zone = hexview::zoneAt(column);
    point.line = cursor.blockNumber();
    point.index = byteIndexAt(zone, column);
    return true;
}

QColor HexPane::mirrorColour(const QPalette &palette) {
    const QColor highlight = palette.color(QPalette::Highlight);
    const QColor base = palette.color(QPalette::Base);
    constexpr int Toward = 66;  // percent of the way to the background
    const auto blend = [](int from, int to) { return from + (to - from) * Toward / 100; };
    return QColor(blend(highlight.red(), base.red()), blend(highlight.green(), base.green()),
                  blend(highlight.blue(), base.blue()));
}

void HexPane::refresh() {
    if (!m_hasSelection) {
        setExtraSelections({});
        return;
    }
    Point first = m_anchor;
    Point last = m_cursor;
    if (last.line < first.line || (last.line == first.line && last.index < first.index)) {
        std::swap(first, last);
    }

    QTextCharFormat highlight;
    highlight.setBackground(palette().highlight());
    highlight.setForeground(palette().highlightedText());

    // The same bytes as the other field reads them. No foreground: the text
    // must stay in its ordinary colour, dimmed nulls included, or the mark
    // reads as a second selection.
    QTextCharFormat mirror;
    mirror.setBackground(mirrorColour(palette()));

    // Selecting offsets is selecting whole lines, and their hex and characters
    // are then both "corresponding" — marking every column of every line says
    // nothing the highlighted offsets have not already said.
    const bool hasMirror = m_zone != Zone::Offset;
    const Zone other = m_zone == Zone::Hex ? Zone::Ascii : Zone::Hex;

    QList<QTextEdit::ExtraSelection> selections;
    for (int number = first.line; number <= last.line; ++number) {
        const QTextBlock block = document()->findBlockByNumber(number);
        if (!block.isValid()) break;
        const QString line = block.text();
        if (!isDumpLine(line)) continue;

        const int from = (number == first.line) ? first.index : 0;
        const int to = (number == last.line) ? last.index : BytesPerLine - 1;

        const auto add = [&](Zone zone, const QTextCharFormat &format) {
            const auto [start, length] = spanOf(zone, from, to);
            QTextEdit::ExtraSelection selection;
            selection.format = format;
            selection.cursor = QTextCursor(block);
            selection.cursor.setPosition(block.position() + start);
            selection.cursor.setPosition(block.position() + qMin(start + length, line.size()),
                                         QTextCursor::KeepAnchor);
            selections.append(selection);
        };
        // The mirror first, so an overlap could only ever be painted over by
        // the selection proper and never the other way about.
        if (hasMirror) add(other, mirror);
        add(m_zone, highlight);
    }
    setExtraSelections(selections);
}

QString HexPane::selectedFieldText() const {
    if (!m_hasSelection) return {};
    Point first = m_anchor;
    Point last = m_cursor;
    if (last.line < first.line || (last.line == first.line && last.index < first.index)) {
        std::swap(first, last);
    }

    QStringList lines;
    for (int number = first.line; number <= last.line; ++number) {
        const QTextBlock block = document()->findBlockByNumber(number);
        if (!block.isValid()) break;
        const QString line = block.text();
        if (!isDumpLine(line)) continue;

        const int from = (number == first.line) ? first.index : 0;
        const int to = (number == last.line) ? last.index : BytesPerLine - 1;
        const auto [start, length] = spanOf(m_zone, from, to);
        // Trailing space only in the hex field, where every byte carries one.
        lines.append(line.mid(start, length).trimmed());
    }
    return lines.join(QChar('\n'));
}

QString HexPane::selectedLines() const {
    if (!m_hasSelection) return {};
    const int first = qMin(m_anchor.line, m_cursor.line);
    const int last = qMax(m_anchor.line, m_cursor.line);

    QStringList lines;
    for (int number = first; number <= last; ++number) {
        const QTextBlock block = document()->findBlockByNumber(number);
        if (!block.isValid()) break;
        lines.append(block.text());
    }
    return lines.join(QChar('\n'));
}

void HexPane::copyLines() {
    const QString text = selectedLines();
    if (!text.isEmpty()) QApplication::clipboard()->setText(text);
}

void HexPane::copyField() {
    const QString text = selectedFieldText();
    if (!text.isEmpty()) QApplication::clipboard()->setText(text);
}

void HexPane::selectField() {
    const int lines = document()->blockCount();
    m_anchor = {0, 0};
    m_cursor = {lines - 1, BytesPerLine - 1};
    m_hasSelection = lines > 0;
    refresh();
}

void HexPane::scrollContentsBy(int dx, int dy) {
    QPlainTextEdit::scrollContentsBy(dx, dy);
    emit scrolled();
}

void HexPane::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) {
        QPlainTextEdit::mousePressEvent(event);
        return;
    }
    Zone zone = Zone::Hex;
    Point point;
    if (!pointAt(event->pos(), zone, point)) {
        m_hasSelection = false;
        refresh();
        return;
    }
    m_zone = zone;
    m_anchor = point;
    m_cursor = point;
    m_selecting = true;
    // A press with no drag is not a selection yet, so a plain click clears
    // whatever was highlighted rather than leaving a one-byte remnant.
    m_hasSelection = false;
    refresh();
    // The real cursor is kept collapsed and moved with the drag: it draws no
    // selection of its own to compete with, and it is what `ensureCursorVisible`
    // scrolls to when a drag runs off the bottom of the pane.
    setTextCursor(cursorForPosition(event->pos()));
}

void HexPane::mouseMoveEvent(QMouseEvent *event) {
    if (!m_selecting) {
        QPlainTextEdit::mouseMoveEvent(event);
        return;
    }
    Zone zone = Zone::Hex;
    Point point;
    // A drag that strays into another field keeps going in the one it started
    // in — the alternative is a selection that changes meaning underneath the
    // pointer, which is the behaviour being fixed.
    if (pointAt(event->pos(), zone, point)) {
        m_cursor = point;
        m_hasSelection = (m_cursor.line != m_anchor.line || m_cursor.index != m_anchor.index);
        refresh();
    }
    QTextCursor moving = cursorForPosition(event->pos());
    setTextCursor(moving);
    ensureCursorVisible();
}

void HexPane::mouseDoubleClickEvent(QMouseEvent *event) {
    Zone zone = Zone::Hex;
    Point point;
    if (!pointAt(event->pos(), zone, point)) {
        QPlainTextEdit::mouseDoubleClickEvent(event);
        return;
    }
    // The whole field on that line, which is the unit a double click means
    // here: a "word" in a hex dump is one line of one column.
    m_zone = zone;
    m_anchor = {point.line, 0};
    m_cursor = {point.line, BytesPerLine - 1};
    m_hasSelection = true;
    refresh();
}

void HexPane::keyPressEvent(QKeyEvent *event) {
    // `copy()` and `selectAll()` are not virtual, so the base class's own
    // shortcut handling cannot be redirected into them — the keys are taken
    // here instead.
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
    QPlainTextEdit::keyPressEvent(event);
}

void HexPane::contextMenuEvent(QContextMenuEvent *event) {
    // The standard menu's Copy calls the non-virtual `copy()`, which would give
    // an empty clipboard: the selection this pane draws is not the document's.
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
