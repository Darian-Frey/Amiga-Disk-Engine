#pragma once

// The hex pane: a dump of bytes, formatted and painted only where it is on
// screen (IMP-017).
//
// The layout constants live beside the pane because everything that places
// something on a line — the null dimming, the selection, the mirror mark, the
// mouse — has to agree about where each byte's characters are. Kept apart, a
// change to one would silently misplace the other, dimming the wrong two
// characters with nothing to say so.

#include <QAbstractScrollArea>
#include <QByteArray>
#include <QColor>
#include <QPalette>
#include <QString>
#include <QTimer>
#include <QVector>

#include <functional>

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

/// Characters in a full dump line: the fields, plus the two `|` around the
/// characters. What the pane has to be wide enough to show.
constexpr int LineLength = AsciiColumn + BytesPerLine + 1;

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

/// One dump line: the offset, up to sixteen bytes, then the printable
/// characters.
QString line(quint64 offset, const QByteArray &bytes);

}  // namespace hexview

/// One run of the disk that is all the same thing, for tinting.
struct HexRegion {
    /// First byte.
    quint64 start;
    /// Last byte, exclusive.
    quint64 end;
    /// An `AdeRegion` code.
    int region;
    /// The block of the owning directory entry, or 0 for none.
    quint32 owner = 0;
    /// The owning entry's path. Carried as well as the block because a row
    /// inside a drawer nobody has opened does not exist to be marked, and the
    /// name is still the answer to "what am I looking at".
    QString path;
};

/// The hex pane.
///
/// # Why it paints its own lines
///
/// It was a `QPlainTextEdit` holding the finished dump, and an 880 KB floppy
/// is 56,000 lines of it: selecting a disk spent 401 ms laying them out and
/// colouring every one, for the forty on screen (IMP-017). Now the pane holds
/// the bytes — or, for a whole disk, a way to read them — and a paint formats
/// and colours only the lines it draws. The cost is a screenful whatever the
/// disk, so the whole-disk view no longer stops at 4 MB.
///
/// The scrollbar counts lines, one step per dump line, so its value *is* the
/// top line. The window relies on that to say which file is on screen.
///
/// # Selection stays inside one field
///
/// A hex dump is three columns pretending to be one line of text, and ordinary
/// text selection does not know that. Dragging down two lines in the hex field
/// would select the end of that line's characters, the ASCII column, the next
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
/// # Null bytes are dimmed, and regions tinted
///
/// Carried over from the Atari Disk Engine. **Only the hex field** is dimmed:
/// the offset column is mostly zeros and the ASCII column can hold the
/// characters `00` from a file that happens to contain them, so the dimming
/// is placed by column rather than by searching the line for `00`.
class HexPane : public QAbstractScrollArea {
    Q_OBJECT

public:
    /// Reads `length` bytes at `offset` of whatever the pane is showing.
    using Reader = std::function<QByteArray(quint64 offset, quint64 length)>;

    /// The document margin, as `QPlainTextEdit` had: the space between the
    /// frame and the first character.
    static constexpr int Margin = 4;

    /// The most a whole-disk view shows: what eight offset digits can name.
    static constexpr quint64 MaxBytes = Q_UINT64_C(1) << 32;

    /// The most bytes one copy takes. Select All on a whole hard disk is
    /// gigabytes, and its dump as clipboard text is five times that; the view
    /// can show it now, but nobody can paste it. Four megabytes is what the
    /// whole-disk view used to stop at, so no copy that worked before is cut.
    static constexpr quint64 CopyLimit = Q_UINT64_C(4) * 1024 * 1024;

    explicit HexPane(QWidget *parent = nullptr);

    /// Show these bytes: a file's.
    void setBytes(const QByteArray &bytes);
    /// Show `size` bytes read through `read` as they come on screen: a whole
    /// disk. `read` must stay valid until the pane is cleared or given
    /// something else. Sizes past [`MaxBytes`] are cut there.
    void setSource(quint64 size, Reader read);
    /// Show a line of text instead of a dump: "(no readable contents)".
    void setMessage(const QString &message);
    /// Show nothing, and forget the selection and the regions.
    void clear();

    /// Tint these runs of the disk. Empty for a file's own bytes, where there
    /// is nothing to say: a file is all one region by definition.
    ///
    /// Must be sorted by `start` and must not overlap, which is what
    /// `ade_layout_open` guarantees.
    void setRegions(QVector<HexRegion> regions);

    /// Whether anything at all is shown, dump or message.
    bool isEmpty() const { return lineCount() == 0; }
    /// How many lines the pane holds.
    int lineCount() const;
    /// The text of line `number`, as it is drawn.
    QString lineText(int number) const;

    /// What is painted on a line beyond its plain text.
    struct Mark {
        enum Kind {
            /// The region's wash, under the whole line.
            Tint,
            /// A null byte in the hex field.
            Dim,
            /// The other field's view of the selected bytes.
            Mirror,
            /// The selection.
            Selection,
        };
        Kind kind;
        int column;
        int length;
        /// Invalid for "leave it as it was".
        QColor background;
        /// Invalid for "leave it as it was".
        QColor foreground;
    };
    /// Every mark on line `number`, in the order it is painted: a later one
    /// wins where two overlap. What the paint draws, so a test can read it.
    QVector<Mark> marksOn(int number) const;

    /// The viewport point over character `column` of line `number`, if it is
    /// on screen: the inverse of what a click resolves to.
    QPoint pointOf(int number, int column) const;

    /// Whether some bytes are selected.
    bool hasSelection() const { return m_hasSelection; }

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

    /// Highlight the byte at `offset` in the hex field, as if it were dragged.
    ///
    /// For arriving at a search hit: scrolling to a line puts the byte on
    /// screen, and this says which of the sixteen it is.
    void selectBytesAt(quint64 offset);

    /// The tint for a region, or an invalid colour for one that is not tinted.
    ///
    /// **Files are deliberately not tinted.** They are most of a disk, and
    /// colouring them colours everything, which is the same as colouring
    /// nothing — the eye has to be able to find the four structural blocks
    /// among the sixteen hundred that hold data. Unclaimed space gets the
    /// faintest wash there is, because "this is not part of any file" is worth
    /// seeing and is also the second most common answer.
    static QColor regionColour(const QPalette &palette, int region);

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

    /// The colour marking the *other* field's view of the selected bytes.
    ///
    /// The highlight, mixed a third of the way into the background. Weaker
    /// than the selection on purpose: the two fields are not equally selected,
    /// and only one of them is what Ctrl+C copies.
    static QColor mirrorColour(const QPalette &palette);

signals:
    /// The view scrolled, by any means.
    void scrolled();

    /// A copy took only the first [`CopyLimit`] bytes of a larger selection.
    void copyCut(quint64 copied, quint64 selected);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void changeEvent(QEvent *event) override;
    void scrollContentsBy(int dx, int dy) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
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

    /// [`marksOn`] for a line whose text is already in hand, so a paint does
    /// not read each line a second time.
    QVector<Mark> marksFor(int number, const QString &text) const;
    /// The bytes of lines `first` to `last` inclusive, in one read.
    QByteArray bytesOf(int first, int last) const;
    /// How many bytes are shown.
    quint64 size() const;
    /// Where a viewport position falls. With `clamp`, a point above or below
    /// the lines is taken to the first or last; without, it is no point.
    bool pointAt(const QPoint &pos, bool clamp, hexview::Zone &zone, Point &point) const;
    /// The selection's two ends, in reading order.
    std::pair<Point, Point> ordered() const;
    /// The selection's last line, as far as one copy takes it.
    int copyEnd(int first, int last) const;
    /// Bytes from the start of line `first` to the end of line `last`.
    quint64 bytesBetween(int first, int last) const;
    /// Say so if a copy of lines `first` to `last` was cut.
    void reportCut(int first, int last);
    /// The region covering `offset`, or -1 for none.
    int regionAt(quint64 offset) const;
    /// Character width and line height in the pane's font. The width is
    /// fractional, as the font's advance is: rounded, every run of a line
    /// would start a little off the grid the next run is drawn on, and lines
    /// of different colours would drift out of alignment with each other.
    qreal charWidth() const;
    int lineHeight() const;
    /// Recompute the scrollbars for the content and the viewport.
    void updateScrollBars();
    /// Extend a drag to `pos`, scrolling if it is past the viewport.
    void dragTo(const QPoint &pos);

    QByteArray m_bytes;
    Reader m_read;
    quint64 m_size = 0;
    QString m_message;
    QVector<HexRegion> m_regions;

    hexview::Zone m_zone = hexview::Zone::Hex;
    Point m_anchor;
    Point m_cursor;
    bool m_selecting = false;
    bool m_hasSelection = false;
    /// Keeps a drag that is held past the top or bottom edge scrolling.
    QTimer m_autoScroll;
    QPoint m_lastDrag;
};
