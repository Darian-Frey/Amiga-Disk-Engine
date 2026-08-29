#pragma once

// Rendering the hex pane: the dump itself, and the dimming of null bytes.
//
// The two live together because they share a layout. The highlighter has to
// know which characters of a line are the hex field, and it can only know that
// from how the dump was written — kept apart, a change to one would silently
// misplace the other, dimming the wrong two characters with nothing to say so.

#include <QPalette>
#include <QPlainTextEdit>
#include <QString>
#include <QSyntaxHighlighter>

class QByteArray;
class QTextDocument;

namespace hexview {

/// Bytes shown per line.
constexpr int BytesPerLine = 16;
/// Where the hex field starts: eight offset digits and two spaces.
constexpr int FieldColumn = 10;
/// Characters per byte column, `"xx "`.
constexpr int ByteWidth = 3;

/// The column at which byte `i` of a line is printed.
///
/// The extra space splitting the sixteen bytes into two groups of eight falls
/// inside this, so nothing else has to know about it.
constexpr int columnOf(int i) {
    return FieldColumn + i * ByteWidth + (i >= BytesPerLine / 2 ? 1 : 0);
}

/// The column at which the printable characters begin, past `" |"`.
constexpr int AsciiColumn = columnOf(BytesPerLine - 1) + ByteWidth + 2;

/// The three fields of a dump line, left to right.
enum class Zone {
    /// The eight-digit offset.
    Offset,
    /// The sixteen hex byte columns.
    Hex,
    /// The printable characters.
    Ascii,
};

/// Which field column `column` falls in.
constexpr Zone zoneAt(int column) {
    if (column < FieldColumn) return Zone::Offset;
    if (column < AsciiColumn - 2) return Zone::Hex;
    return Zone::Ascii;
}

/// A classic hex dump: offset, sixteen bytes, then the printable characters.
QString dump(const QByteArray &data);

}  // namespace hexview

/// Dims the `00` pairs in the hex field, so real data stands out from the
/// padding and empty space that fills most of a disk.
///
/// Carried over from the Atari Disk Engine, whose hex view paints itself and
/// picks the colour with a hand-written light/dark test. ADE's is a
/// `QPlainTextEdit`, so the same effect comes from a highlighter over its
/// document — cheaper, too, since Qt only ever re-highlights the lines on
/// screen rather than all 4,096 of a 64 KB preview.
///
/// **Only the hex field.** The offset column is mostly zeros and the ASCII
/// column can hold the characters `00` from a file that happens to contain
/// them, so the dimming is placed by column arithmetic rather than by
/// searching each line for the text `00`. A search would dim `00000000` at the
/// start of every line, which is the opposite of making data stand out.
class HexNullDimmer : public QSyntaxHighlighter {
public:
    /// Dims nulls in `document`, taking its colour from `source`'s palette.
    ///
    /// The palette is read at highlight time rather than captured, so the
    /// dimming follows a theme change instead of staying the old theme's grey.
    HexNullDimmer(QTextDocument *document, const QPlainTextEdit *source);

    /// The dim colour for a given palette.
    ///
    /// Blended 70% of the way from the text colour toward the background,
    /// rather than being two constants for "light" and "dark". On a standard
    /// light theme that gives (178,178,178) against the Atari engine's
    /// hand-picked (180,180,185); on the dark backgrounds desktops actually
    /// use it gives 97 to 105 against their 100. Close enough to read as the
    /// same choice, and unlike a pair of constants it stays legible on a
    /// palette neither of them anticipated.
    static QColor dimColour(const QPalette &palette);

protected:
    void highlightBlock(const QString &line) override;

private:
    const QPlainTextEdit *m_source;
};

/// The hex pane, whose selection stays inside one field.
///
/// # Why this is not a plain `QPlainTextEdit`
///
/// A hex dump is three columns pretending to be one line of text, and ordinary
/// text selection does not know that. Dragging down two lines in the hex field
/// selects the end of that line's characters, the ASCII column, the next
/// line's offset, and only then more hex — so copying a screenful of hex is
/// impossible, which is one of the two things anybody does with a hex view.
///
/// So a drag is clamped to the field it started in. Within a line the result
/// is what selecting text always did; across lines it is the run of bytes
/// between the two points, which is what the display is *about*. Copying gives
/// back what was highlighted and nothing else — hex without offsets, or
/// characters without hex — one line of clipboard text per line on screen.
///
/// # The other field is marked too
///
/// Hex and characters are two readings of the same bytes, and which bytes is
/// the question a hex view exists to answer. So selecting hex marks the
/// characters those bytes spell, and selecting characters marks their hex.
///
/// The mark is **deliberately weaker than the selection**: a paler wash, with
/// the text colour left alone. Painted identically the two fields would look
/// equally selected, and nothing on screen would say which of them Ctrl+C is
/// about to copy — replacing one ambiguity with a worse one.
///
/// # What this does not do
///
/// Keyboard selection with shift and the arrow keys still moves the ordinary
/// text cursor, which is not clamped. Making that agree would mean
/// reimplementing cursor movement over a layout the document does not
/// describe; the drag is where the problem actually bites.
class HexPane : public QPlainTextEdit {
    Q_OBJECT

public:
    explicit HexPane(QWidget *parent = nullptr);

    /// The highlighted text, as it would reach the clipboard.
    ///
    /// Empty when nothing is selected. One line per line on screen, with each
    /// line trimmed to the selected part of its field.
    QString selectedFieldText() const;

    /// The whole dump lines the selection spans, offsets and all.
    ///
    /// Clamping the drag takes away the one way there was to copy a line as it
    /// appears on screen, which is what somebody pasting into a bug report
    /// wants. This puts it back somewhere findable rather than behind a
    /// modifier key nobody would guess.
    QString selectedLines() const;

    /// Put [`selectedFieldText`] on the clipboard.
    void copyField();

    /// Put [`selectedLines`] on the clipboard.
    void copyLines();

    /// Select every line, across the whole of the last field used.
    void selectField();

    /// The colour marking the *other* field's view of the selected bytes.
    ///
    /// The highlight, mixed a third of the way into the background. Weaker
    /// than the selection on purpose: the two fields are not equally selected,
    /// and only one of them is what Ctrl+C copies.
    static QColor mirrorColour(const QPalette &palette);

protected:
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void contextMenuEvent(QContextMenuEvent *event) override;

private:
    /// One end of a selection: a line, and an index within a field.
    struct Point {
        int line = 0;
        /// A byte column in the hex and ASCII fields; unused for the offset.
        int index = 0;
    };

    /// Where a viewport position falls, or nothing if it is past the text.
    bool pointAt(const QPoint &pos, hexview::Zone &zone, Point &point) const;
    /// Repaint the selection from the current anchor and cursor.
    void refresh();

    hexview::Zone m_zone = hexview::Zone::Hex;
    Point m_anchor;
    Point m_cursor;
    bool m_selecting = false;
    bool m_hasSelection = false;
};
