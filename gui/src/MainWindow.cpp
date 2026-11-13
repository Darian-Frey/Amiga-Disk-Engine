#include "MainWindow.h"

#include "HexView.h"
#include "MapView.h"
#include "SurfaceView.h"
#include "SegmentBar.h"
#include "GraphView.h"
#include "ImageTree.h"

#include <QAction>
#include <QApplication>
#include <functional>
#include <QMouseEvent>
#include <QKeyEvent>
#include <QToolButton>
#include <QShortcut>
#include <QCheckBox>
#include <QSyntaxHighlighter>
#include <QCloseEvent>
#include <QInputDialog>
#include <set>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QFile>
#include <QDir>
#include <QDialog>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QSpinBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QFontMetrics>
#include <QGuiApplication>
#include <QComboBox>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QScreen>
#include <QStyle>
#include <QLabel>
#include <QPushButton>
#include <QIcon>
#include <QMessageBox>
#include <QMenuBar>
#include <QMimeData>
#include <QPlainTextEdit>
#include <QScrollBar>
#include <QTreeWidgetItemIterator>
#include <QSplitter>
#include <QStatusBar>
#include <QTabWidget>
#include <QTextBrowser>
#include <QLineEdit>
#include <QTimer>
#include <QTreeWidget>
#include <QUrl>
#include <QVBoxLayout>

namespace {
using namespace tree;

// The Assembly view's colouring (F-032): remarks and headings in a muted
// colour, labels bold, data dimmed. Derived from the widget's own palette, so
// it reads in a dark theme as well as a light one — the same rule the hex
// pane's tints follow.
class AsmHighlighter : public QSyntaxHighlighter {
public:
    AsmHighlighter(QTextDocument *document, const QWidget *owner)
        : QSyntaxHighlighter(document) {
        const QPalette p = owner->palette();
        const QColor text = p.color(QPalette::Text);
        const QColor base = p.color(QPalette::Base);
        const auto mix = [&](qreal t) {
            return QColor::fromRgbF(text.redF() * t + base.redF() * (1 - t),
                                    text.greenF() * t + base.greenF() * (1 - t),
                                    text.blueF() * t + base.blueF() * (1 - t));
        };
        m_remark.setForeground(mix(0.55));
        m_remark.setFontItalic(true);
        m_label.setFontWeight(QFont::Bold);
        m_data.setForeground(mix(0.6));
        m_address.setForeground(mix(0.5));
        // Where control goes, as the Atari engine's listing showed it: a hue
        // half mixed into the text colour, so it reads on either theme.
        const auto hue = [&](int r, int g, int b) {
            return QColor::fromRgbF((r / 255.0 + text.redF()) / 2, (g / 255.0 + text.greenF()) / 2,
                                    (b / 255.0 + text.blueF()) / 2);
        };
        m_flow[ADE_FLOW_CALL].setForeground(hue(60, 120, 230));
        m_flow[ADE_FLOW_RETURN].setForeground(hue(40, 170, 80));
        m_flow[ADE_FLOW_JUMP].setForeground(hue(60, 120, 230));
        m_flow[ADE_FLOW_BRANCH].setForeground(hue(40, 160, 190));
        m_flow[ADE_FLOW_TRAP].setForeground(hue(220, 150, 30));
        m_flow[ADE_FLOW_LIBRARY].setForeground(hue(170, 80, 200));
        m_flow[ADE_FLOW_CALL].setFontWeight(QFont::DemiBold);
        m_flow[ADE_FLOW_LIBRARY].setFontWeight(QFont::DemiBold);
    }

    /// How to learn where control goes from the line in block `n`.
    void setFlowOf(std::function<AdeAsmFlow(int)> flowOf) { m_flowOf = std::move(flowOf); }

protected:
    void highlightBlock(const QString &line) override {
        if (line.startsWith(QLatin1Char(';'))) {
            setFormat(0, static_cast<int>(line.size()), m_remark);
            return;
        }
        if (line.endsWith(QLatin1Char(':'))) {
            setFormat(0, static_cast<int>(line.size()), m_label);
            return;
        }
        // Address and bytes: the first two columns, 36 characters.
        constexpr int Columns = 36;
        setFormat(0, qMin(Columns, static_cast<int>(line.size())), m_address);
        if (line.mid(Columns).startsWith(QStringLiteral("dc."))) {
            setFormat(Columns, static_cast<int>(line.size()) - Columns, m_data);
        }
        const auto comment = line.indexOf(QStringLiteral("; "), Columns);
        const AdeAsmFlow flow = m_flowOf ? m_flowOf(currentBlock().blockNumber()) : ADE_FLOW_NONE;
        if (flow != ADE_FLOW_NONE && line.size() > Columns) {
            const int end = comment > 0 ? static_cast<int>(comment) : static_cast<int>(line.size());
            setFormat(Columns, end - Columns, m_flow[flow]);
        }
        if (comment > 0) {
            setFormat(static_cast<int>(comment), static_cast<int>(line.size() - comment), m_remark);
        }
    }

private:
    QTextCharFormat m_remark, m_label, m_data, m_address;
    QTextCharFormat m_flow[ADE_FLOW_LIBRARY + 1];
    std::function<AdeAsmFlow(int)> m_flowOf;
};


// The icon for a kind of file (an `AdeFileKind`, -1 for a folder): the
// desktop theme's, so it matches the person's other file views, and the
// style's plain file or folder where the theme has none.
QIcon kindIcon(int kind) {
    const char *name = nullptr;
    switch (kind) {
        case -1:                    name = "folder"; break;
        case ADE_FILE_PROGRAM:      name = "application-x-executable"; break;
        case ADE_FILE_LIBRARY:      name = "application-x-sharedlib"; break;
        case ADE_FILE_FONT:         name = "font-x-generic"; break;
        case ADE_FILE_KEYMAP:       name = "input-keyboard"; break;
        case ADE_FILE_PRINTER:      name = "printer"; break;
        case ADE_FILE_ICON:         name = "preferences-desktop-icons"; break;
        case ADE_FILE_PICTURE:      name = "image-x-generic"; break;
        case ADE_FILE_ANIMATION:    name = "video-x-generic"; break;
        case ADE_FILE_SOUND:        name = "audio-x-generic"; break;
        case ADE_FILE_MUSIC:        name = "audio-x-generic"; break;
        case ADE_FILE_TEXT:         name = "text-x-generic"; break;
        case ADE_FILE_CATALOG:      name = "preferences-desktop-locale"; break;
        case ADE_FILE_PREFERENCES:  name = "preferences-system"; break;
        case ADE_FILE_DATATYPE:     name = "preferences-system"; break;
        case ADE_FILE_PACKED:       name = "package-x-generic"; break;
        case ADE_FILE_ARCHIVE:      name = "package-x-generic"; break;
        case ADE_FILE_DISK_IMAGE:   name = "media-floppy"; break;
        default:                    name = "text-x-generic-template"; break;
    }
    const QIcon fallback = QApplication::style()->standardIcon(
        kind == -1 ? QStyle::SP_DirIcon : QStyle::SP_FileIcon);
    return QIcon::fromTheme(QString::fromLatin1(name), fallback);
}

// How much of a file to show. A preview is a preview: reading a whole 512 KB
// file into a text widget to look at its first line is a poor trade.
constexpr int PreviewBytes = 64 * 1024;

// An Amiga datestamp rendered for people. Day 0 means "unset", which Amiga
// software treats as blank rather than as 1978.
QString formatDate(quint32 days, quint32 mins, quint32 ticks) {
    if (days == 0 && mins == 0 && ticks == 0) return {};
    const QDate epoch(1978, 1, 1);
    const QDate date = epoch.addDays(static_cast<qint64>(days));
    const QTime time = QTime(0, 0).addSecs(static_cast<int>(mins) * 60 +
                                           static_cast<int>(ticks) / 50);
    return QDateTime(date, time).toString(QStringLiteral("yyyy-MM-dd HH:mm:ss"));
}

// The Amiga's `hsparwed` string. Owner bits 0-3 are set to *forbid*, so a
// cleared bit permits — the inversion is the thing people get wrong.
QString formatProtection(quint32 bits) {
    const QString flags = QStringLiteral("hsparwed");
    QString out;
    for (int i = 0; i < 8; ++i) {
        const bool owner = i >= 4;
        const int bit = 7 - i;
        const bool set = (bits >> bit) & 1;
        const bool shown = owner ? !set : set;
        out += shown ? flags[i] : QChar('-');
    }
    return out;
}

// How wide the hex pane has to be to show a whole dump line.
//
// Measured from the pane's own font and frame rather than guessed: the font is
// whatever the desktop calls fixed-width, which is not the same size on two
// machines, and a guess that works here is the bug being fixed somewhere else.
int hexPaneWidth(const HexPane &pane) {
    const QFontMetrics metrics(pane.font());
    const int text = metrics.horizontalAdvance(QString(hexview::LineLength, QChar('0')));
    // The pane's own left and right margins, the frame, and room for the
    // vertical scrollbar a full disk will always need.
    const int margins = HexPane::Margin * 2;
    const int frame = pane.frameWidth() * 2;
    const int scrollbar = pane.style()->pixelMetric(QStyle::PM_ScrollBarExtent);
    return text + margins + frame + scrollbar + 4;
}

// How wide the tree has to be for a filename and its three measured columns.
int treePaneWidth(const QTreeWidget &tree) {
    const QFontMetrics metrics(tree.font());
    // Long enough for the names real disks carry: "INSTALL PROGRAM.info" and
    // "4GETLEVELS.ORIG" are both from one corpus image, and a name is indented
    // by its depth in the tree.
    int width = metrics.horizontalAdvance(QStringLiteral("INSTALL PROGRAM.info")) + 60;
    for (int column = 1; column < tree.columnCount(); ++column) {
        width += tree.columnWidth(column);
    }
    return width + tree.frameWidth() * 2 +
           tree.style()->pixelMetric(QStyle::PM_ScrollBarExtent);
}

}  // namespace

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle(QStringLiteral("Amiga Disk Engine"));
    setAcceptDrops(true);

    m_tree = new ImageTree(this);
    m_tree->setObjectName(QStringLiteral("tree"));
    // Host files dropped onto a disk's row, or a folder on it, are copied
    // onto that disk (D-018); dropped on empty space they are opened as disks.
    m_tree->setAcceptsFiles(true);
    connect(m_tree, &ImageTree::filesDropped, this, &MainWindow::filesDropped);
    connect(m_tree, &ImageTree::rowsDropped, this,
            [this](QTreeWidgetItem *target, const QList<QTreeWidgetItem *> &rows) {
                moveItems(target, rows);
            });
    m_tree->setColumnCount(4);
    m_tree->setHeaderLabels({QStringLiteral("Name"), QStringLiteral("Size"),
                             QStringLiteral("Modified"), QStringLiteral("Protection")});
    m_tree->header()->setStretchLastSection(false);
    m_tree->header()->setSectionResizeMode(ColName, QHeaderView::Stretch);
    // The other three hold fixed-width content — a size, a timestamp, and
    // eight protection flags — and are sized to fit the widest value each can
    // hold, measured **once**.
    //
    // `ResizeToContents` is the obvious way to do this and is quadratic: Qt
    // re-measures every row in the tree on every insertion, so the cost of
    // expanding a drawer grows with the whole tree rather than with the
    // drawer. Measured over 60 images and 3,827 rows, it was **35.6 seconds**
    // against 27 milliseconds — a thousandfold, and the entire reason the
    // window felt slow at scale (IMP-008).
    //
    // Measuring the content shape instead costs nothing and gives the same
    // answer, because these columns do not vary: a datestamp is always
    // nineteen characters, protection always eight, and a size on an 880 KB
    // disk never exceeds seven digits.
    const QFontMetrics metrics(m_tree->font());
    const int padding = 24;
    m_tree->header()->setSectionResizeMode(ColSize, QHeaderView::Fixed);
    m_tree->header()->setSectionResizeMode(ColDate, QHeaderView::Fixed);
    m_tree->header()->setSectionResizeMode(ColProtection, QHeaderView::Fixed);
    m_tree->setColumnWidth(ColSize, metrics.horizontalAdvance(QStringLiteral("999999999")) + padding);
    m_tree->setColumnWidth(
        ColDate, metrics.horizontalAdvance(QStringLiteral("1990-09-20 17:10:20")) + padding);
    m_tree->setColumnWidth(ColProtection,
                           metrics.horizontalAdvance(QStringLiteral("hsparwed")) + padding);
    m_tree->setUniformRowHeights(true);
    // Banded, as the Atari engine's tree was: long listings are easier to
    // follow across four columns.
    m_tree->setAlternatingRowColors(true);

    m_hex = new HexPane(this);
    m_hex->setObjectName(QStringLiteral("hex"));
    m_hex->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    // Scrolling the whole disk says which file is on screen. Not by selecting
    // its row: selection is what *chose* the whole-disk view, so following the
    // scroll with it would replace the view being scrolled — the feature would
    // undo itself on the first wheel click. The row is marked instead, which
    // is a different thing said a different way.
    connect(m_hex, &HexPane::scrolled, this, [this] { markWhatIsOnScreen(); });
    // A copy of a whole hard disk is cut rather than attempted; saying so is
    // the difference between a limit and a clipboard that is quietly short.
    connect(m_hex, &HexPane::copyCut, this, [this](quint64 copied, quint64 selected) {
        statusBar()->showMessage(
            QStringLiteral("Copied the first %1 of %2 selected bytes").arg(copied).arg(selected));
    });

    // And a backstop, because Qt does not reliably say when the view moved.
    // Measured: a click in the scrollbar trough scrolled the pane from line 0
    // to line 37 — the painted text proves it moved — while emitting
    // `valueChanged` zero times and calling `scrollContentsBy` zero times. The
    // wheel and a drag of the handle both notify, which is why the gap reads
    // as the feature working until somebody pages down.
    //
    // So the position is also polled. It costs one integer comparison per
    // tick: `markWhatIsOnScreen` returns immediately unless the top line has
    // actually changed, and the timer only runs while a whole disk is shown.
    m_follow = new QTimer(this);
    m_follow->setInterval(120);
    connect(m_follow, &QTimer::timeout, this, [this] { markWhatIsOnScreen(); });

    m_text = new QPlainTextEdit(this);
    m_text->setReadOnly(true);
    m_text->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));

    // Results drag out too — the same widget, so a file found by searching
    // behaves exactly like a file found by browsing.
    m_results = new ImageTree(this);
    m_results->setObjectName(QStringLiteral("results"));
    m_results->setColumnCount(3);
    m_results->setHeaderLabels({QStringLiteral("Name"), QStringLiteral("Path"),
                                QStringLiteral("Image")});
    m_results->setRootIsDecorated(false);
    m_results->header()->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    m_results->header()->setSectionResizeMode(1, QHeaderView::Stretch);
    m_results->header()->setSectionResizeMode(2, QHeaderView::ResizeToContents);

    // The legend for the region tints, under the hex pane and only when there
    // is something to explain. A colour nobody can name is decoration; the
    // Atari engine puts its legend behind a menu item, which means it is read
    // once and never found again.
    m_legend = new QLabel(this);
    m_legend->setObjectName(QStringLiteral("legend"));
    m_legend->setTextFormat(Qt::RichText);
    m_legend->setWordWrap(true);
    m_legend->setContentsMargins(4, 2, 4, 2);
    m_legend->hide();

    m_hexTab = new QWidget(this);
    auto *hexTab = m_hexTab;
    auto *hexLayout = new QVBoxLayout(hexTab);
    hexLayout->setContentsMargins(0, 0, 0, 0);
    hexLayout->setSpacing(0);
    hexLayout->addWidget(m_hex);
    hexLayout->addWidget(m_legend);

    m_views = new QTabWidget(this);
    // The map, with its own legend: its colours are not the hex pane's,
    // because there files are left untinted and here they must not be.
    m_map = new MapView(this);
    m_map->setObjectName(QStringLiteral("map"));
    connect(m_map, &MapView::blockChosen, this, [this](quint64 block) {
        goToOffset(block * 512);
    });

    m_mapLegend = new QLabel(this);
    m_mapLegend->setObjectName(QStringLiteral("mapLegend"));
    m_mapLegend->setTextFormat(Qt::RichText);
    m_mapLegend->setWordWrap(true);
    m_mapLegend->setContentsMargins(4, 2, 4, 2);
    m_mapLegend->hide();

    m_mapTab = new QWidget(this);
    auto *mapLayout = new QVBoxLayout(m_mapTab);
    mapLayout->setContentsMargins(4, 4, 4, 0);
    mapLayout->setSpacing(0);
    mapLayout->addWidget(m_map);
    mapLayout->addWidget(m_mapLegend);

    m_views->addTab(hexTab, QStringLiteral("Hex"));
    m_views->addTab(m_mapTab, QStringLiteral("Map"));
    m_views->addTab(m_text, QStringLiteral("Text"));

    // The Assembly view (F-032): a program's 68000 code, or the disk's boot
    // code. Built only while it is showing, so browsing with it hidden costs
    // nothing.
    m_asm = new QPlainTextEdit(this);
    m_asm->setObjectName(QStringLiteral("assembly"));
    m_asm->setReadOnly(true);
    m_asm->setLineWrapMode(QPlainTextEdit::NoWrap);
    m_asm->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    auto *highlighter = new AsmHighlighter(m_asm->document(), m_asm);
    highlighter->setFlowOf([this](int block) {
        return block >= 0 && block < m_asmShown.lines.size() ? m_asmShown.lines[block].flow
                                                            : ADE_FLOW_NONE;
    });
    // Moving about the listing: double-click or Enter follows, Alt+arrows go
    // back and forward (the filter), and a strip below says what refers to
    // the line the cursor is on.
    m_asm->viewport()->installEventFilter(this);
    m_asm->installEventFilter(this);
    connect(m_asm, &QPlainTextEdit::cursorPositionChanged, this,
            [this] { showAssemblyReferrers(); });
    // Naming a place and commenting on a line (D-022), from the line itself.
    m_asm->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(m_asm, &QWidget::customContextMenuRequested, this, [this](const QPoint &at) {
        const int line = m_asm->cursorForPosition(at).blockNumber();
        QMenu *menu = m_asm->createStandardContextMenu();
        const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
        const bool placed = view < 0 && !m_asmForceRaw && line >= 0 &&
                            line < m_asmShown.lines.size() && m_asmShown.lines[line].hasOffset;
        menu->addSeparator();
        QAction *flowGraph = menu->addAction(QStringLiteral("Control flow of this function (G)"));
        QAction *callGraph = menu->addAction(QStringLiteral("Call graph"));
        flowGraph->setEnabled(placed);
        callGraph->setEnabled(view < 0 && !m_asmForceRaw && !m_asmShown.lines.isEmpty());
        connect(flowGraph, &QAction::triggered, this, [this, line] { drawAssemblyGraph(line, false); });
        connect(callGraph, &QAction::triggered, this, [this, line] { drawAssemblyGraph(line, true); });
        menu->addSeparator();
        QAction *name = menu->addAction(QStringLiteral("Name this place..."));
        QAction *note = menu->addAction(QStringLiteral("Comment on this line..."));
        name->setEnabled(placed);
        note->setEnabled(placed);
        const auto ask = [this, line](bool comment) {
            bool ok = false;
            const QString text = QInputDialog::getText(
                this, comment ? QStringLiteral("Comment") : QStringLiteral("Name"),
                comment ? QStringLiteral("Comment on this line (empty to clear):")
                        : QStringLiteral("Name for this place (empty to clear):"),
                QLineEdit::Normal, QString(), &ok);
            if (ok) annotateAssemblyLine(line, text.trimmed(), comment);
        };
        connect(name, &QAction::triggered, this, [ask] { ask(false); });
        connect(note, &QAction::triggered, this, [ask] { ask(true); });
        menu->exec(m_asm->viewport()->mapToGlobal(at));
        delete menu;
    });
    m_asmStatus = new QLabel(this);
    m_asmStatus->setObjectName(QStringLiteral("assemblyStatus"));
    m_asmStatus->setWordWrap(true);
    m_asmStatus->setContentsMargins(4, 2, 4, 2);
    m_asmRaw = new QPushButton(QStringLiteral("Decode anyway"), this);
    m_asmRaw->setObjectName(QStringLiteral("assemblyRaw"));
    m_asmRaw->setToolTip(QStringLiteral(
        "Decode the file from its first byte as if it were 68000 code.\n"
        "Instructions are 2 to 10 bytes long, so if this is not code, or the code\n"
        "starts elsewhere, the result is noise that looks like code."));
    m_asmRaw->hide();
    connect(m_asmRaw, &QPushButton::clicked, this, [this] {
        m_asmForceRaw = true;
        m_asmStale = true;
        refreshAssembly();
    });
    m_asmTab = new QWidget(this);
    auto *asmBar = new QHBoxLayout;
    // What each instruction does, at the end of its line (F-032). On by
    // default: the listing is for reading, and the column is what makes it
    // readable to somebody who does not already know 68000. Turned off, the
    // listing is a plain assembler's — and nothing is decoded again.
    m_asmExplain = new QCheckBox(QStringLiteral("Explain"), this);
    m_asmExplain->setObjectName(QStringLiteral("assemblyExplain"));
    m_asmExplain->setChecked(true);
    m_asmExplain->setToolTip(QStringLiteral(
        "Describe what each instruction does, and name the Amiga hardware it touches."));
    connect(m_asmExplain, &QCheckBox::toggled, this, [this] { renderAssembly(); });
    // The views beside the listing (D-021), found by following the code from
    // where it starts rather than by reading every byte as code.
    m_asmShow = new QComboBox(this);
    m_asmShow->setObjectName(QStringLiteral("assemblyShow"));
    m_asmShow->addItem(QStringLiteral("Listing"), -1);
    m_asmShow->addItem(QStringLiteral("Functions"), int(ADE_ASM_FUNCTIONS));
    m_asmShow->addItem(QStringLiteral("Cross-references"), int(ADE_ASM_REFERENCES));
    m_asmShow->addItem(QStringLiteral("Call graph"), int(ADE_ASM_CALL_GRAPH));
    m_asmShow->addItem(QStringLiteral("Strings"), int(ADE_ASM_STRINGS));
    m_asmShow->addItem(QStringLiteral("Variables"), int(ADE_ASM_VARIABLES));
    m_asmShow->addItem(QStringLiteral("Source"), int(ADE_ASM_SOURCE));
    m_asmShow->setToolTip(QStringLiteral(
        "Listing: every byte of the code, decoded.\n"
        "Functions, Cross-references, Call graph, Strings, Variables: what following\n"
        "the code from where it starts finds. A library call is named only where the\n"
        "code proves which library it calls.\n"
        "Source: the program as assembler source that builds back into the same bytes."));
    connect(m_asmShow, &QComboBox::currentIndexChanged, this, [this] {
        m_asmStale = true;
        refreshAssembly();
    });
    m_asmSave = new QPushButton(QStringLiteral("Save..."), this);
    m_asmSave->setObjectName(QStringLiteral("assemblySave"));
    m_asmSave->setToolTip(QStringLiteral(
        "Save what is shown as a text file: the source as .s, the call graph for Graphviz "
        "as .dot."));
    m_asmSave->setEnabled(false);
    connect(m_asmSave, &QPushButton::clicked, this, [this] { saveAssembly(); });
    asmBar->addWidget(m_asmStatus, 1);
    asmBar->addWidget(m_asmShow);
    asmBar->addWidget(m_asmExplain);
    asmBar->addWidget(m_asmSave);
    asmBar->addWidget(m_asmRaw);
    auto *asmLayout = new QVBoxLayout(m_asmTab);
    asmLayout->setContentsMargins(0, 0, 0, 0);
    asmLayout->setSpacing(0);
    asmLayout->addLayout(asmBar);
    // The program to scale beside its listing (F-036); a click goes there.
    m_asmBar = new SegmentBar(this);
    m_asmBar->setObjectName(QStringLiteral("assemblySegments"));
    m_asmBar->hide();
    connect(m_asmBar, &SegmentBar::jumpTo, this,
            [this](quint64 offset) { showAssemblyOffset(offset); });
    auto *asmBody = new QHBoxLayout;
    asmBody->setContentsMargins(0, 0, 0, 0);
    asmBody->setSpacing(2);
    asmBody->addWidget(m_asm, 1);
    asmBody->addWidget(m_asmBar);
    asmLayout->addLayout(asmBody, 1);
    m_asmRefs = new QLabel(this);
    m_asmRefs->setObjectName(QStringLiteral("assemblyReferrers"));
    m_asmRefs->setTextFormat(Qt::RichText);
    m_asmRefs->setWordWrap(true);
    m_asmRefs->setContentsMargins(4, 2, 4, 2);
    m_asmRefs->hide();
    connect(m_asmRefs, &QLabel::linkActivated, this,
            [this](const QString &line) { goToAssemblyLine(line.toInt()); });
    asmLayout->addWidget(m_asmRefs);
    // Back and forward, beside the view selector.
    m_asmBack = new QToolButton(this);
    m_asmBack->setObjectName(QStringLiteral("assemblyBack"));
    m_asmBack->setArrowType(Qt::LeftArrow);
    m_asmBack->setToolTip(QStringLiteral("Back (Alt+Left)"));
    m_asmBack->setEnabled(false);
    connect(m_asmBack, &QToolButton::clicked, this, [this] { stepAssemblyHistory(-1); });
    m_asmForward = new QToolButton(this);
    m_asmForward->setObjectName(QStringLiteral("assemblyForward"));
    m_asmForward->setArrowType(Qt::RightArrow);
    m_asmForward->setToolTip(QStringLiteral("Forward (Alt+Right)"));
    m_asmForward->setEnabled(false);
    connect(m_asmForward, &QToolButton::clicked, this, [this] { stepAssemblyHistory(1); });
    asmBar->insertWidget(1, m_asmBack);
    asmBar->insertWidget(2, m_asmForward);
    // Go to a name, and find text, while the Assembly tab has focus.
    auto *goTo = new QShortcut(QKeySequence(QStringLiteral("Ctrl+G")), m_asmTab);
    goTo->setContext(Qt::WidgetWithChildrenShortcut);
    connect(goTo, &QShortcut::activated, this, [this] {
        bool ok = false;
        const QString name = QInputDialog::getText(
            this, QStringLiteral("Go to"),
            QStringLiteral("A name — a label, h1_0042, a symbol or your own:"), QLineEdit::Normal,
            QString(), &ok);
        if (ok && !goToAssemblyName(name)) {
            emit errorOccurred(QStringLiteral("Nothing in this listing is called %1.").arg(name));
        }
    });
    // Ctrl+F is the window's search across every image; finding in the
    // listing takes Ctrl+Shift+F, and F3 again, set by key: the platform's
    // "find next" can include Ctrl+G, which is Go to here.
    auto *findHere = new QShortcut(QKeySequence(QStringLiteral("Ctrl+Shift+F")), m_asmTab);
    findHere->setContext(Qt::WidgetWithChildrenShortcut);
    connect(findHere, &QShortcut::activated, this, [this] {
        bool ok = false;
        const QString text = QInputDialog::getText(this, QStringLiteral("Find"),
                                                   QStringLiteral("Find in the listing:"),
                                                   QLineEdit::Normal, m_asmFind, &ok);
        if (ok && !text.isEmpty()) {
            m_asmFind = text;
            findInAssembly();
        }
    });
    auto *again = new QShortcut(QKeySequence(Qt::Key_F3), m_asmTab);
    again->setContext(Qt::WidgetWithChildrenShortcut);
    connect(again, &QShortcut::activated, this, [this] { findInAssembly(); });
    m_views->addTab(m_asmTab, QStringLiteral("Assembly"));

    // The Decoded view (F-033): a file ADE recognises, in words. Built only
    // while it is showing, like the Assembly view.
    m_decoded = new QTextBrowser(this);
    m_decoded->setObjectName(QStringLiteral("decoded"));
    m_decoded->setOpenLinks(false);
    m_views->addTab(m_decoded, QStringLiteral("Decoded"));

    connect(m_views, &QTabWidget::currentChanged, this, [this] {
        fillVisiblePreview();
        if (m_views->currentWidget() == m_asmTab) refreshAssembly();
        if (m_views->currentWidget() == m_decoded) refreshDecoded();
    });

    m_views->addTab(m_results, QStringLiteral("Search"));

    // The search box sits above the tree: it searches every open image, not
    // the selected one, so it belongs to the window rather than to a disk.
    m_query = new QLineEdit(this);
    m_query->setPlaceholderText(QStringLiteral("Search all open images by name..."));
    m_query->setClearButtonEnabled(true);
    connect(m_query, &QLineEdit::returnPressed, this, &MainWindow::search);

    // Names or contents, beside the box rather than as a second box. They are
    // the same question asked of two different things, and two boxes would
    // leave one of them stale and wrong-looking whenever the other was used.
    m_mode = new QComboBox(this);
    m_mode->setObjectName(QStringLiteral("mode"));
    m_mode->addItem(QStringLiteral("Names"));
    m_mode->addItem(QStringLiteral("Contents"));
    m_mode->setToolTip(QStringLiteral(
        "Names searches every filename on every open disk.\n"
        "Contents searches the bytes of every open disk, including the parts "
        "no file occupies."));
    connect(m_mode, &QComboBox::currentIndexChanged, this, [this] { updateSearchHint(); });
    updateSearchHint();

    auto *queryRow = new QWidget(this);
    auto *queryLayout = new QHBoxLayout(queryRow);
    queryLayout->setContentsMargins(0, 0, 0, 0);
    queryLayout->addWidget(m_query);
    queryLayout->addWidget(m_mode);

    auto *left = new QWidget(this);
    auto *leftLayout = new QVBoxLayout(left);
    leftLayout->setContentsMargins(0, 0, 0, 0);
    leftLayout->addWidget(queryRow);
    leftLayout->addWidget(m_tree);

    auto *splitter = new QSplitter(Qt::Horizontal, this);
    splitter->addWidget(left);
    splitter->addWidget(m_views);
    // The tree keeps the width its columns need; every extra pixel goes to the
    // pane showing the disk. An even split instead sounds fair and is not: the
    // tree's content has a natural width and the hex pane's does not, so half
    // the window is more than the tree can use and less than the dump needs.
    splitter->setStretchFactor(0, 0);
    splitter->setStretchFactor(1, 1);
    setCentralWidget(splitter);

    // The window opens at the size its contents need, rather than at a number
    // somebody typed. 1100x700 with an even split gave the hex pane about 550
    // pixels for a line that measures 78 monospaced characters, so the
    // characters column was cut off on the very first disk anybody opened —
    // and the fix was to resize the window by hand every time.
    //
    // Both halves are measured the same way the tree's fixed columns already
    // are: from the text they have to hold, in the font they will hold it in.
    const int hexWidth = hexPaneWidth(*m_hex);
    const int treeWidth = treePaneWidth(*m_tree);
    const int handle = splitter->handleWidth();

    // Clamped to the screen, because a measurement is not permission to be
    // bigger than the display — that is the same fault in the other
    // direction, and worse, since a window wider than the desktop can put its
    // own edges out of reach.
    // The height stays a choice rather than a measurement: a disk is taller
    // than any window, so there is no content height to fit and 700 is simply
    // a comfortable number of lines.
    QSize wanted(treeWidth + hexWidth + handle, 700);
    if (const QScreen *screen = QGuiApplication::primaryScreen()) {
        const QSize room = screen->availableGeometry().size() - QSize(80, 80);
        wanted = wanted.boundedTo(room.expandedTo(QSize(640, 400)));
    }
    resize(wanted);

    // After the resize, not before: a splitter redistributes a resize among
    // its children, so sizes set first are proportions the next resize spends.
    //
    // When the screen could not hold both, the tree is the one that gives way.
    // Its content degrades gracefully — a long filename elides and is still a
    // filename — where a clipped dump line is simply missing, which is the
    // complaint that started this. It keeps a floor wide enough to read a name
    // in, because a browser narrowed to nothing is not a trade, it is a
    // different bug.
    const int available = wanted.width() - handle;
    const int floor = qMin(treeWidth, 240);
    const int hexShare = qBound(available / 3, available - floor, hexWidth);
    splitter->setSizes({available - hexShare, hexShare});

    // The tree extracts through the window, which owns the images; the tree
    // itself knows nothing about them.
    const auto extractor = [this](QTreeWidgetItem *item) { return contentsOf(item); };
    m_tree->setExtractor(extractor);
    m_results->setExtractor(extractor);

    m_summary = new QLabel(this);
    statusBar()->addPermanentWidget(m_summary);

    auto *file = menuBar()->addMenu(QStringLiteral("&File"));
    auto *blank = file->addAction(QStringLiteral("&New disk..."));
    blank->setShortcut(QKeySequence::New);
    connect(blank, &QAction::triggered, this, &MainWindow::newDisk);

    auto *open = file->addAction(QStringLiteral("&Open image..."));
    open->setShortcut(QKeySequence::Open);
    connect(open, &QAction::triggered, this, &MainWindow::chooseImage);

    // Saving (D-018). The first save of an edited disk is always a Save As:
    // ADE never writes over the file a disk was opened from.
    m_save = file->addAction(QStringLiteral("&Save"));
    m_save->setShortcut(QKeySequence::Save);
    m_save->setEnabled(false);
    connect(m_save, &QAction::triggered, this, &MainWindow::save);
    m_saveAs = file->addAction(QStringLiteral("Save &as..."));
    m_saveAs->setShortcut(QKeySequence::SaveAs);
    m_saveAs->setEnabled(false);
    connect(m_saveAs, &QAction::triggered, this, &MainWindow::saveAs);

    m_extractAll = file->addAction(QStringLiteral("Extract &all files..."));
    m_extractAll->setEnabled(false);
    connect(m_extractAll, &QAction::triggered, this, &MainWindow::extractAll);

    m_extract = file->addAction(QStringLiteral("&Extract selected..."));
    m_extract->setShortcut(QKeySequence(Qt::CTRL | Qt::Key_E));
    m_extract->setEnabled(false);
    connect(m_extract, &QAction::triggered, this, &MainWindow::extractSelected);

    file->addSeparator();
    auto *find = file->addAction(QStringLiteral("&Search all images"));
    find->setShortcut(QKeySequence::Find);
    connect(find, &QAction::triggered, this, [this] {
        m_query->setFocus();
        m_query->selectAll();
    });

    auto *closeAllAction = file->addAction(QStringLiteral("&Close all"));
    connect(closeAllAction, &QAction::triggered, this, &MainWindow::closeAll);

    auto *quit = file->addAction(QStringLiteral("&Quit"));
    quit->setShortcut(QKeySequence::Quit);
    connect(quit, &QAction::triggered, this, &QWidget::close);

    // Edits are in memory until saved, so undo is a step back through copies
    // of the disk rather than a second write that might fail (D-018).
    auto *edit = menuBar()->addMenu(QStringLiteral("&Edit"));
    m_undo = edit->addAction(QStringLiteral("&Undo"));
    m_undo->setShortcut(QKeySequence::Undo);
    m_undo->setEnabled(false);
    connect(m_undo, &QAction::triggered, this, &MainWindow::undoEdit);
    m_redo = edit->addAction(QStringLiteral("&Redo"));
    m_redo->setShortcut(QKeySequence::Redo);
    m_redo->setEnabled(false);
    connect(m_redo, &QAction::triggered, this, &MainWindow::redoEdit);
    edit->addSeparator();
    m_newFolder = edit->addAction(QStringLiteral("New &folder..."));
    m_newFolder->setShortcut(QKeySequence(Qt::CTRL | Qt::SHIFT | Qt::Key_N));
    m_newFolder->setEnabled(false);
    m_newFolder->setToolTip(QStringLiteral(
        "Make a folder in the selected folder, or on the selected disk.\n"
        "To copy files on, drop them onto the disk or a folder in the tree."));
    connect(m_newFolder, &QAction::triggered, this, &MainWindow::newFolder);
    m_rename = edit->addAction(QStringLiteral("Re&name..."));
    m_rename->setShortcut(QKeySequence(Qt::Key_F2));
    m_rename->setEnabled(false);
    connect(m_rename, &QAction::triggered, this, &MainWindow::renameSelected);
    m_makeLink = edit->addAction(QStringLiteral("Make hard &link..."));
    m_makeLink->setEnabled(false);
    m_makeLink->setToolTip(QStringLiteral(
        "Make a hard link to the selected file or folder, beside it, as AmigaDOS's\n"
        "MakeLink HARD does. Deleting the original keeps the contents under the link."));
    connect(m_makeLink, &QAction::triggered, this, [this] {
        if (!m_selected) return;
        bool ok = false;
        const QString name = QInputDialog::getText(
            this, QStringLiteral("Make hard link"), QStringLiteral("Name of the link"),
            QLineEdit::Normal, QStringLiteral("Link to %1").arg(m_selected->text(ColName)), &ok);
        if (ok && !name.isEmpty()) makeLinkTo(m_selected, name);
    });
    m_delete = edit->addAction(QStringLiteral("&Delete"));
    m_delete->setShortcut(QKeySequence::Delete);
    m_delete->setEnabled(false);
    m_delete->setToolTip(QStringLiteral(
        "Delete the selected file or folder. Like AmigaDOS, ADE frees its blocks without\n"
        "erasing them, so Carve can still find it; Undo brings it back until you save."));
    connect(m_delete, &QAction::triggered, this, &MainWindow::deleteSelected);

    m_confirmSave = [this](const QString &question) {
        return QMessageBox::question(this, QStringLiteral("Save changes"), question,
                                     QMessageBox::Save | QMessageBox::Cancel,
                                     QMessageBox::Save) == QMessageBox::Save;
    };
    m_confirmDelete = [this](const QString &question) {
        return QMessageBox::question(this, QStringLiteral("Delete"), question,
                                     QMessageBox::Yes | QMessageBox::Cancel,
                                     QMessageBox::Cancel) == QMessageBox::Yes;
    };
    m_confirmWipe = [this](const QString &question) {
        return QMessageBox::question(this, QStringLiteral("Wipe free space"), question,
                                     QMessageBox::Yes | QMessageBox::Cancel,
                                     QMessageBox::Cancel) == QMessageBox::Yes;
    };
    m_confirmFreeData = [this](const QString &question) {
        return QMessageBox::warning(this, QStringLiteral("Free blocks hold data"), question,
                                    QMessageBox::Yes | QMessageBox::Cancel,
                                    QMessageBox::Cancel) == QMessageBox::Yes;
    };
    m_confirmDiscard = [this](const QString &question) {
        return QMessageBox::question(this, QStringLiteral("Unsaved changes"), question,
                                     QMessageBox::Discard | QMessageBox::Cancel,
                                     QMessageBox::Cancel) == QMessageBox::Discard;
    };

    // A menu of its own, because what a disk *is* and what it *needs* are the
    // window's two standing questions about the thing on screen, and neither
    // belongs under File.
    auto *disk = menuBar()->addMenu(QStringLiteral("&Disk"));
    m_specs = disk->addAction(QStringLiteral("Disk &information..."));
    m_specs->setEnabled(false);
    connect(m_specs, &QAction::triggered, this, &MainWindow::showSpecs);

    m_surface = disk->addAction(QStringLiteral("Disk &surface..."));
    m_surface->setEnabled(false);
    m_surface->setToolTip(QStringLiteral(
        "What came off each track of the medium.\n"
        "Only an extended ADF or a flux capture records this; a plain ADF is "
        "already sectors and never knew."));
    connect(m_surface, &QAction::triggered, this, &MainWindow::showSurface);

    // Recovery sits here rather than under File because it is a question
    // about the disk — what is on it that nothing points at — and not a way of
    // getting at files the tree already lists.
    m_carve = disk->addAction(QStringLiteral("&Recover lost files..."));
    m_carve->setEnabled(false);
    m_carve->setToolTip(QStringLiteral(
        "Files no directory entry reaches: what a deletion leaves behind.\n"
        "Works on disks that do not mount, which are the ones worth carving."));
    connect(m_carve, &QAction::triggered, this, &MainWindow::recoverLostFiles);

    // An edit, so it goes on the undo stack and waits for a save like any
    // other — but an edit of the disk as a whole, which is why it is here.
    m_repair = disk->addAction(QStringLiteral("Re&pair disk"));
    m_repair->setEnabled(false);
    m_repair->setToolTip(QStringLiteral(
        "Mark every block a file uses as used, seal the bitmap and set its valid flag; "
        "move entries filed under the wrong hash slot to the right one.\n"
        "Blocks nothing reaches stay marked used: a game may keep data there."));
    connect(m_repair, &QAction::triggered, this, &MainWindow::repairSelected);

    // Edits of the disk as a whole, like Repair: undoable until saved, and
    // saved only to a file of the user's choosing.
    m_format = disk->addAction(QStringLiteral("&Format..."));
    m_format->setEnabled(false);
    m_format->setToolTip(QStringLiteral(
        "A quick format, as AmigaDOS's: an empty disk of the same size and filesystem.\n"
        "The old blocks are left as they were, free, so Recover lost files can still "
        "find them."));
    connect(m_format, &QAction::triggered, this, &MainWindow::formatSelected);
    m_wipeSlack = disk->addAction(QStringLiteral("Wipe file &slack"));
    m_wipeSlack->setEnabled(false);
    m_wipeSlack->setToolTip(QStringLiteral(
        "Zero the bytes after each file's last, in its last block: whatever an older "
        "version of it left there.\nNo file changes."));
    connect(m_wipeSlack, &QAction::triggered, this, &MainWindow::wipeSlackSelected);
    m_wipeFree = disk->addAction(QStringLiteral("Wipe free spac&e..."));
    m_wipeFree->setEnabled(false);
    m_wipeFree->setToolTip(QStringLiteral(
        "Zero file slack and every free block. Nothing deleted can be recovered after, "
        "and a game that reads blocks directly may keep data in blocks its bitmap calls "
        "free."));
    connect(m_wipeFree, &QAction::triggered, this, &MainWindow::wipeFreeSelected);

    // Help. The manual will join About here; nothing stands in for it in the
    // meantime, because a menu item that is greyed out or opens an apology is
    // a promise the window has already broken.
    auto *help = menuBar()->addMenu(QStringLiteral("&Help"));
    auto *about = help->addAction(QStringLiteral("&About Amiga Disk Engine"));
    about->setMenuRole(QAction::AboutRole);
    connect(about, &QAction::triggered, this, &MainWindow::showAbout);

    // Only one of the two trees holds the selection at a time. Leaving a row
    // highlighted in the tree while the views show a search result invites
    // reading the preview as belonging to the highlighted row — the two are
    // usually different files, and the byte count is the only clue.
    const auto takeSelection = [this](ImageTree *from, ImageTree *other) {
        if (m_syncing) return;
        const auto selected = from->selectedItems();
        if (!selected.isEmpty()) {
            m_syncing = true;
            other->clearSelection();
            m_syncing = false;
        }
        showEntry(selected.isEmpty() ? nullptr : selected.first());
        updateEditActions();
    };
    connect(m_tree, &QTreeWidget::itemSelectionChanged, this,
            [this, takeSelection] { takeSelection(m_tree, m_results); });
    connect(m_results, &QTreeWidget::itemSelectionChanged, this,
            [this, takeSelection] { takeSelection(m_results, m_tree); });
    connect(m_tree, &QTreeWidget::itemExpanded, this, [this](QTreeWidgetItem *item) {
        if (item->data(0, RolePopulated).toBool()) return;
        item->setData(0, RolePopulated, true);
        const Open *open = imageFor(item);
        if (open) {
            populate(item, *open, item->data(0, RolePartition).toUInt(),
                     item->data(0, RoleBlock).toUInt());
        }
    });

    // Identification on open (F-013). Loading the dataset takes about 140 ms,
    // so it happens once here rather than per image — and only when one is
    // configured, which costs nothing when it is not.
    const QString datfiles = ade::Catalogue::configuredLocation();
    if (!datfiles.isEmpty()) {
        m_catalogue = ade::Catalogue::load(datfiles);
    }
    statusBar()->showMessage(
        m_catalogue
            ? QStringLiteral("%1 dataset entries loaded — open a disk image, or drop one here")
                  .arg(m_catalogue.count())
            : QStringLiteral("Open a disk image, or drop one here"));
}

void MainWindow::chooseImage() {
    const QString path = QFileDialog::getOpenFileName(
        this, QStringLiteral("Open a disk image"), {},
        QStringLiteral("Disk images (*.adf *.adz *.hdf *.hdz *.dms);;All files (*)"));
    if (!path.isEmpty()) openImage(path);
}

void MainWindow::openImage(const QString &path) {
    ade::Image image = ade::Image::open(path, m_catalogue ? &m_catalogue : nullptr);
    if (!image) {
        emit errorOccurred(QStringLiteral("%1: ADE could not read this file (error %2).")
                               .arg(QFileInfo(path).fileName())
                               .arg(static_cast<int>(image.error())));
        return;
    }

    // A file that is no kind of disk image is declined rather than shown as a
    // row explaining that it could not be read. This is not the same as having
    // no volume: a DMS archive or an IPF is recognised and unmountable, and
    // opening it to say so is the point (IMP-006).
    //
    // Found by dragging three files out of a disk and dropping them back on
    // the window, which opened two Amiga executables and a level file as
    // damaged hard disks (BUG-010).
    if (!image.recognised()) {
        // Said with the way to do what was probably meant: a file dropped on
        // empty space is opened, and one dropped onto a disk is copied onto it.
        emit errorOccurred(QStringLiteral("%1 is not a disk image. To copy it onto a disk, "
                                          "drop it onto the disk or one of its folders.")
                               .arg(QFileInfo(path).fileName()));
        return;
    }

    auto open = std::make_unique<Open>();
    open->image = std::move(image);
    open->path = path;
    open->name = QFileInfo(path).fileName();
    open->index = m_images.size();
    m_images.push_back(std::move(open));

    addImageRoot(*m_images.back());
    showSummary();
    statusBar()->showMessage(QStringLiteral("Opened %1").arg(m_images.back()->name));
}

bool MainWindow::closeAll() {
    if (!mayDiscardEdits()) return false;
    // The trees first: their items hold indices into `m_images`, and an item
    // outliving the image it points at is the one way this can dangle.
    m_tree->clear();
    m_results->clear();
    m_selected = nullptr;
    // The pane before the images: a whole-disk view reads through a pointer
    // to its image, and must not be left holding one that is gone.
    m_hex->clear();
    m_images.clear();
    clearViews();
    // Image numbers start again from 0, so the map's must not linger.
    m_map->clear();
    m_mapLegend->hide();
    m_mapImage = NoMap;
    setWindowTitle(QStringLiteral("Amiga Disk Engine"));
    m_summary->clear();
    m_lastEdited = static_cast<size_t>(-1);
    updateEditActions();
    statusBar()->showMessage(QStringLiteral("Open a disk image, or drop one here"));
    return true;
}

void MainWindow::addImageRoot(Open &open) {
    // Each image is a root in the tree. Even with one open this costs a click,
    // but it is what lets a second image be opened without displacing the
    // first — and search results are only meaningful once that is true.
    auto *root = new QTreeWidgetItem(m_tree);
    root->setData(0, RoleImage, static_cast<qulonglong>(open.index));
    root->setData(0, RoleIsDir, true);
    root->setData(0, RolePopulated, true);

    // An image is not a file, and its columns are not a file's columns: an
    // image has no size, no datestamp and no protection bits. Writing the
    // container into "Modified" because the space was free reads as though the
    // disk were last modified in "ADF (DD, 80 cylinders)". So the row spans
    // instead, and says what an image has.
    root->setText(ColName, QStringLiteral("%1   %2").arg(open.name, describe(open)));
    root->setFirstColumnSpanned(true);
    // A TOSEC filename is eighty characters, so even a spanned row runs out
    // and elides the container off the end. The tooltip cannot elide, and
    // selecting the row puts the same line in the status bar.
    // The dataset's name goes in the tooltip beside the path: the row itself
    // is already carrying the file, the container and the volume, and a TOSEC
    // name is eighty characters on its own.
    root->setToolTip(ColName, rootToolTip(open));

    QFont bold = m_tree->font();
    bold.setBold(true);
    root->setFont(ColName, bold);

    // A hard disk holds no volume of its own: every volume is inside a
    // partition, so the partitions are a level of the tree rather than
    // something to choose between in a menu. Nothing else in the window
    // changes — a partition is just another thing with files under it.
    const ade::Partitions partitions = open.image.partitions();
    if (partitions) {
        for (size_t i = 0; i < partitions.count(); ++i) {
            AdePartition partition{};
            if (!partitions.at(i, &partition)) continue;
            addPartition(root, open, static_cast<quint32>(i), partition);
        }
        root->setExpanded(true);
        return;
    }

    if (!open.image.hasVolume()) {
        addKindRow(root, open);
        root->setExpanded(true);
        return;
    }
    root->setData(0, RolePartition, ADE_WHOLE_IMAGE);
    populate(root, open, ADE_WHOLE_IMAGE, open.image.rootBlock());
    addKindRow(root, open);
    root->setExpanded(true);
}

void MainWindow::addKindRow(QTreeWidgetItem *root, const Open &open) {
    // Only the kinds a person would otherwise read as broken: a blank disk is
    // plainly blank, and an unreadable one already says why on its row.
    QString where;
    switch (open.image.kind()) {
    case ADE_KIND_OWN_LOADER:
        where = QStringLiteral("its boot code is in the Assembly tab, and the whole disk in the "
                               "Hex tab");
        break;
    case ADE_KIND_DATA_DISK:
    case ADE_KIND_GAME_BEHIND_EMPTY_ROOT:
        where = QStringLiteral("the whole disk is in the Hex tab");
        break;
    case ADE_KIND_CUSTOM_TRACKS:
        where = QStringLiteral("what came off each track is under Disk ▸ Disk surface");
        break;
    default:
        return;
    }
    auto *note = new QTreeWidgetItem(root);
    // The row above says what the disk is; this says where to look instead,
    // and the tooltip why.
    note->setText(ColName, QStringLiteral("Nothing to list here: %1").arg(where));
    note->setToolTip(ColName, open.image.kindExplanation());
    note->setFirstColumnSpanned(true);
    // Enabled, so its tooltip shows, but not selectable, draggable or a drop
    // target: it is the window speaking, not something on the disk.
    note->setFlags(Qt::ItemIsEnabled);
    QFont italic = note->font(ColName);
    italic.setItalic(true);
    note->setFont(ColName, italic);
    if (const QTreeWidget *tree = root->treeWidget()) {
        note->setForeground(ColName, tree->palette().brush(QPalette::Disabled, QPalette::Text));
    }
}

QString MainWindow::rootToolTip(const Open &open) {
    QStringList lines{open.path};
    const QString destination = open.image.saveDestination();
    if (!destination.isEmpty()) lines << QStringLiteral("saved as %1").arg(destination);
    const QString named = open.image.identified();
    if (!named.isEmpty()) lines << named;
    lines << describe(open);
    // The plain words are on the row; the evidence they came from is here,
    // for anybody who wants to know why ADE says so.
    const QString absent = open.image.volumeAbsent();
    if (!open.image.hasVolume() && !absent.isEmpty() && open.image.kind() != ADE_KIND_UNREADABLE) {
        lines << absent;
    }
    return lines.join(QLatin1Char('\n'));
}

// What an image is, in one line: container, volume, size, and anything the
// health check found. Shown on the image's row, in its tooltip, in the status
// bar when it is selected, and as the summary when it is the only one open —
// four places that must not be able to disagree.
QString MainWindow::describe(const Open &open) {
    QStringList parts;
    parts << open.image.container();
    // A device holds no volume of its own, and saying "no rootblock at block
    // 4096" about a perfectly sound hard disk reads as damage. What it has is
    // partitions, so that is what it says. The engine draws the same
    // distinction — a report calling a device volumeless is calling a working
    // disk broken.
    const ade::Partitions partitions = open.image.partitions();
    if (partitions) {
        const size_t count = partitions.count();
        size_t mounted = 0;
        for (size_t i = 0; i < count; ++i) {
            AdePartition partition{};
            if (partitions.at(i, &partition) && partition.mounts) ++mounted;
        }
        parts << QStringLiteral("%1 partition%2, %3 mounting")
                     .arg(count)
                     .arg(count == 1 ? "" : "s")
                     .arg(mounted);
    } else {
        // A missing volume is not an error here either: a quarter of real
        // images are not AmigaDOS disks, and the container is still worth
        // showing.
        // Without a volume, what the disk *is* says more than why it has no
        // volume: "no rootblock at block 880" is true of a quarter of real
        // disks and explains none of them. The technical reason stays in the
        // tooltip. An unreadable container keeps its own reason, which is
        // the more specific of the two.
        const AdeDiskKind kind = open.image.kind();
        if (open.image.hasVolume()) {
            QString volume = QStringLiteral("\"%1\"").arg(open.image.volumeName());
            if (kind != ADE_KIND_FILES) volume += QStringLiteral(" — %1").arg(open.image.kindSummary());
            parts << volume;
        } else {
            parts << (kind == ADE_KIND_UNREADABLE ? open.image.volumeAbsent()
                                                  : open.image.kindSummary());
        }
    }
    parts << QStringLiteral("%1 bytes").arg(open.image.size());
    const size_t findings = open.image.findingCount();
    if (findings > 0) {
        parts << QStringLiteral("%1 finding%2").arg(findings).arg(findings == 1 ? "" : "s");
    }
    // Said in words on the row as well as with the title's asterisk: with
    // several disks open, the title cannot say which one.
    if (open.image.isModified()) parts << QStringLiteral("unsaved changes");
    return parts.join(QStringLiteral("   "));
}

void MainWindow::addPartition(QTreeWidgetItem *root, const Open &open, quint32 index,
                              const AdePartition &partition) {
    auto *item = new QTreeWidgetItem(root);
    const QString name = ade::latin1(partition.name);
    item->setData(0, RoleImage, static_cast<qulonglong>(open.index));
    item->setData(0, RolePartition, index);
    item->setData(0, RoleIsDir, true);
    item->setData(0, RolePopulated, true);
    item->setFirstColumnSpanned(true);

    QStringList parts{name};
    if (partition.mounts) {
        parts << QStringLiteral("\"%1\"").arg(ade::latin1(partition.volume_name));
    } else {
        // Saying so beats showing an empty drawer. A partition ADE cannot read
        // is a real partition — `PFS\0` and `SFS\0` exist — and an empty
        // listing would read as an empty disk.
        parts << QStringLiteral("no AmigaDOS volume");
    }
    parts << QStringLiteral("%1 blocks of %2 bytes")
                 .arg(partition.blocks)
                 .arg(partition.block_size);
    if (partition.bootable) parts << QStringLiteral("bootable");
    item->setText(ColName, parts.join(QStringLiteral("   ")));

    QFont bold = m_tree->font();
    bold.setBold(true);
    item->setFont(ColName, bold);

    if (!partition.mounts) return;
    item->setData(0, RoleBlock, partition.root_block);
    populate(item, open, index, partition.root_block);
    item->setExpanded(true);
}

void MainWindow::showSummary() {
    if (m_images.empty()) {
        m_summary->clear();
        return;
    }
    if (m_images.size() > 1) {
        m_summary->setText(QStringLiteral("%1 images open").arg(m_images.size()));
        setWindowTitle(QStringLiteral("%1 images — Amiga Disk Engine").arg(m_images.size()));
        return;
    }
    const Open &only = *m_images.front();
    m_summary->setText(describe(only));
    setWindowTitle(QStringLiteral("%1%2 — Amiga Disk Engine")
                       .arg(only.image.isModified() ? QStringLiteral("*") : QString{}, only.name));
}

std::vector<std::pair<quint32, QString>> MainWindow::volumesOf(const Open &open) {
    std::vector<std::pair<quint32, QString>> out;
    const ade::Partitions partitions = open.image.partitions();
    if (partitions) {
        for (size_t i = 0; i < partitions.count(); ++i) {
            AdePartition partition{};
            if (!partitions.at(i, &partition) || !partition.mounts) continue;
            out.emplace_back(static_cast<quint32>(i),
                             QStringLiteral("%1 — %2")
                                 .arg(open.name, ade::latin1(partition.name)));
        }
        return out;
    }
    if (open.image.hasVolume()) out.emplace_back(ADE_WHOLE_IMAGE, open.name);
    return out;
}

MainWindow::Open *MainWindow::imageFor(const QTreeWidgetItem *item) {
    return const_cast<Open *>(std::as_const(*this).imageFor(item));
}

const MainWindow::Open *MainWindow::imageFor(const QTreeWidgetItem *item) const {
    if (!item) return nullptr;
    const qulonglong index = item->data(0, RoleImage).toULongLong();
    if (index >= m_images.size()) return nullptr;
    return m_images[index].get();
}

void MainWindow::populate(QTreeWidgetItem *parent, const Open &open, quint32 partition,
                          quint32 block) {
    const ade::Listing listing = open.image.list(partition, block);
    if (!listing) return;

    for (size_t i = 0; i < listing.count(); ++i) {
        AdeEntry entry{};
        if (!listing.entry(i, &entry)) continue;

        const bool isDir =
            entry.kind == ADE_ENTRY_DIRECTORY || entry.kind == ADE_ENTRY_LINK_DIR;
        // The volume met as an entry of itself (IMP-015): shown, because it
        // is on the disk, and never a folder — opening it would list the disk
        // inside itself, one level deeper on every click.
        const bool isVolume = entry.kind == ADE_ENTRY_VOLUME;
        auto *item = parent ? new QTreeWidgetItem(parent) : new QTreeWidgetItem(m_tree);
        item->setText(ColName, ade::latin1(entry.name));
        item->setText(ColSize, isDir     ? QString{}
                               : isVolume ? QStringLiteral("volume")
                                          : QString::number(entry.size));
        if (isVolume) {
            item->setToolTip(ColName,
                             QStringLiteral("This volume's own rootblock: a hash table leads back to "
                                            "it, so the disk is listed as an entry of itself. It "
                                            "holds no file data."));
        }
        item->setText(ColDate, formatDate(entry.days, entry.mins, entry.ticks));
        item->setText(ColProtection, formatProtection(entry.protection));
        item->setTextAlignment(ColSize, Qt::AlignRight | Qt::AlignVCenter);
        item->setData(0, RoleBlock, entry.block);
        item->setData(0, RoleIsDir, isDir);
        item->setData(0, RoleImage, static_cast<qulonglong>(open.index));
        item->setData(0, RolePartition, partition);
        // What it is, at a glance, as the Atari engine's tree showed: the
        // engine names the kind from the file's contents.
        if (isDir) {
            item->setIcon(ColName, kindIcon(-1));
        } else if (!isVolume) {
            const quint32 kind = open.image.fileKind(partition, entry.block);
            item->setIcon(ColName, kindIcon(static_cast<int>(kind)));
            item->setToolTip(ColName, QStringLiteral("%1 — %2").arg(
                                          item->text(ColName),
                                          QString::fromUtf8(ade_file_kind_name(kind))));
        }

        if (isDir) {
            // A placeholder child makes the expander appear; the real children
            // are read when it is opened. Walking a whole disk up front is
            // wasted work on an image the user only wants to glance at.
            item->setData(0, RolePopulated, false);
            item->setChildIndicatorPolicy(QTreeWidgetItem::ShowIndicator);
        }
    }
    // Sort what was just read, not the whole tree: `sortItems` reaches every
    // level, which would reorder the images themselves and leave them in
    // alphabetical order rather than the order they were opened in.
    if (parent) {
        parent->sortChildren(ColName, Qt::AscendingOrder);
    } else {
        m_tree->sortItems(ColName, Qt::AscendingOrder);
    }
}

/// The placeholder and the results columns follow the mode.
void MainWindow::updateSearchHint() {
    const bool contents = m_mode && m_mode->currentIndex() == 1;
    m_query->setPlaceholderText(
        contents ? QStringLiteral("Search the bytes of all open images: text, or hex like 60 1A...")
                 : QStringLiteral("Search all open images by name..."));
    m_results->setHeaderLabels(contents
                                   ? QStringList{QStringLiteral("Offset"), QStringLiteral("Where"),
                                                 QStringLiteral("Image")}
                                   : QStringList{QStringLiteral("Name"), QStringLiteral("Path"),
                                                 QStringLiteral("Image")});
}

void MainWindow::search() {
    m_results->clear();
    const QString query = m_query->text().trimmed();
    if (query.isEmpty()) {
        statusBar()->showMessage(m_mode && m_mode->currentIndex() == 1
                                     ? QStringLiteral("Type text or hex to search the disks for")
                                     : QStringLiteral("Type a name to search for"));
        return;
    }
    if (m_mode && m_mode->currentIndex() == 1) {
        searchContents(query);
        return;
    }

    int matches = 0;
    int volumes = 0;
    for (const auto &open : m_images) {
        // Every volume of every image: on a hard disk that is one search per
        // partition, and searching only the first would quietly miss most of
        // the disk.
        for (const auto &[partition, where] : volumesOf(*open)) {
            // The engine walks the disk. Doing it here would mean
            // reimplementing cycle detection in the GUI, and a crafted image
            // would hang the window (AV-001).
            const ade::Listing all = open->image.walk(partition);
            if (!all) continue;
            ++volumes;

            for (size_t i = 0; i < all.count(); ++i) {
                AdeEntry entry{};
                if (!all.entry(i, &entry)) continue;
                const QString name = ade::latin1(entry.name);
                if (!name.contains(query, Qt::CaseInsensitive)) continue;

                const bool isDir =
                    entry.kind == ADE_ENTRY_DIRECTORY || entry.kind == ADE_ENTRY_LINK_DIR;
                auto *item = new QTreeWidgetItem(m_results);
                item->setText(0, name);
                item->setText(1, ade::latin1(entry.path));
                item->setText(2, where);
                item->setData(0, RoleBlock, entry.block);
                item->setData(0, RoleIsDir, isDir);
                item->setData(0, RoleImage, static_cast<qulonglong>(open->index));
                item->setData(0, RolePartition, partition);
                // Both of these outrun their columns — a TOSEC filename is 80
                // characters and a path can be deeper than it is wide.
                item->setToolTip(1, item->text(1));
                item->setToolTip(2, open->path);
                ++matches;
            }
        }
    }

    m_views->setCurrentWidget(m_results);
    // Volumes rather than images: on a hard disk the two differ, and "3
    // images" would undercount what was actually searched.
    statusBar()->showMessage(QStringLiteral("%1 match%2 for \"%3\" across %4 volume%5 in %6 "
                                            "image%7")
                                 .arg(matches)
                                 .arg(matches == 1 ? "" : "es")
                                 .arg(query)
                                 .arg(volumes)
                                 .arg(volumes == 1 ? "" : "s")
                                 .arg(m_images.size())
                                 .arg(m_images.size() == 1 ? "" : "s"));
}

/// Mark the tree row for whatever the hex pane is showing, and name it.
///
/// Only in the whole-disk view: a file's own bytes are all one file, so there
/// is nothing to follow.
void MainWindow::markWhatIsOnScreen() {
    if (m_diskRegions.isEmpty()) return;

    // Nothing to do unless the view actually moved. This is what makes polling
    // cheap enough to be the backstop for Qt's missing notifications.
    if (m_hex->verticalScrollBar()->value() == m_topLine) return;
    m_topLine = m_hex->verticalScrollBar()->value();

    // The top visible line, taken from the scrollbar rather than by asking
    // which line is under the top-left corner of the viewport.
    //
    // `cursorForPosition` was the obvious way and is wrong here: it reads where
    // the viewport has been *painted*, and `valueChanged` fires before
    // QPlainTextEdit has scrolled. Dragging the handle happened to work, and
    // the wheel happened to work, but a click in the scrollbar trough moved
    // the bar from line 0 to line 37 and reported line 0 — and stayed wrong,
    // because nothing scrolled again to correct it.
    //
    // The scrollbar's value is not an approximation of the top line: with word
    // wrap off, QPlainTextEdit's vertical scrollbar counts blocks, and one
    // block is one dump line. It is the thing that changed, so it cannot be
    // stale.
    const int line = m_hex->verticalScrollBar()->value();
    const quint64 offset = static_cast<quint64>(line) * hexview::BytesPerLine;

    const HexRegion *span = nullptr;
    int low = 0;
    int high = m_diskRegions.size() - 1;
    while (low <= high) {
        const int mid = low + (high - low) / 2;
        const HexRegion &at = m_diskRegions.at(mid);
        if (offset < at.start) {
            high = mid - 1;
        } else if (offset >= at.end) {
            low = mid + 1;
        } else {
            span = &at;
            break;
        }
    }
    if (!span) return;

    markRow(span->owner);

    // The name comes from the map, not from the row that was marked: a file
    // inside a drawer nobody has opened has no row, and "file" on its own says
    // nothing at all. The mark is the bonus; the name is the answer.
    QString where = QString::fromUtf8(ade_region_name(static_cast<AdeRegion>(span->region)));
    if (!span->path.isEmpty()) where = QStringLiteral("%1  %2").arg(where, span->path);
    statusBar()->showMessage(QStringLiteral("offset %1   %2").arg(offset).arg(where));
}

/// Mark the row for entry `block`, and unmark whatever was marked before.
///
/// Zero means nothing owns these bytes, which unmarks. A row that has not been
/// realised yet — a file inside a drawer nobody has opened — simply is not
/// found; the status bar still names the region, and the tree is deliberately
/// **not** expanded to reveal it, because a tree that reorganises itself under
/// the pointer while you scroll is worse than one that does not.
void MainWindow::markRow(quint32 block) {
    if (m_marked) {
        QFont plain = m_marked->font(ColName);
        plain.setBold(false);
        m_marked->setFont(ColName, plain);
        for (int column = 0; column < m_tree->columnCount(); ++column) {
            m_marked->setBackground(column, QBrush());
        }
        m_marked = nullptr;
    }
    if (block == 0) return;

    // Scoped to the image and partition being viewed. A block number is unique
    // within a volume and nowhere else, so an unscoped search marks a row in
    // whichever disk happens to hold that block first — and with several images
    // open, scrolling one would quietly highlight a file in another.
    QTreeWidgetItem *found = nullptr;
    for (QTreeWidgetItemIterator it(m_tree); *it; ++it) {
        if ((*it)->data(0, RoleBlock).toUInt() != block) continue;
        if ((*it)->data(0, RoleImage).toULongLong() != m_diskImage) continue;
        if ((*it)->data(0, RolePartition).toUInt() != m_diskPartition) continue;
        found = *it;
        break;
    }
    if (!found) return;

    // The same wash the hex pane marks the other field with — and **bold** as
    // well, because the wash alone is a paler version of the selection colour
    // and the selected row is right there in the same tree. Rendered side by
    // side the two read alike; bold is the part that cannot be mistaken for a
    // slightly different blue.
    const QBrush wash(HexPane::mirrorColour(m_tree->palette()));
    for (int column = 0; column < m_tree->columnCount(); ++column) {
        found->setBackground(column, wash);
    }
    QFont bold = found->font(ColName);
    bold.setBold(true);
    found->setFont(ColName, bold);
    m_marked = found;
    m_tree->scrollToItem(found, QAbstractItemView::EnsureVisible);
}

QString MainWindow::aboutTitle() {
    // The version comes from the engine, like every other fact this window
    // shows. A version string written in Qt is one that can disagree with the
    // library it was built against, which is the most misleading thing an
    // About box can do — it is the one place people go to find out exactly
    // what they are running.
    return QStringLiteral("<h3>Amiga Disk Engine %1</h3>"
                          "<p>Reads, verifies, catalogues and converts Amiga floppy "
                          "and hard-disk images.</p>")
        .arg(QString::fromUtf8(ade_version()).toHtmlEscaped());
}

QString MainWindow::aboutDetail() {
    // "No third-party code" is a claim NOTICE makes and this repeats, so it has
    // to stay true: if D-009 ever brings xDMS in, this line changes with it.
    return QStringLiteral(
               "<p>Apache License 2.0. Contains no third-party code: the filesystem, "
               "the containers, the checksums and the MFM codec are all written from "
               "their specifications.</p>"
               "<p style='color:gray'>Qt %1 &middot; "
               "<a href=\"https://github.com/Darian-Frey/Amiga-Disk-Engine\">"
               "github.com/Darian-Frey/Amiga-Disk-Engine</a></p>")
        .arg(QString::fromLatin1(qVersion()));
}

void MainWindow::searchContents(const QString &query) {
    const QByteArray pattern = query.toUtf8();
    int hits = 0;
    QString refused;

    for (const auto &open : m_images) {
        const ade::Search found = open->image.find(pattern.constData());
        if (!found) continue;
        // A refused pattern is not a search that found nothing, and the two
        // must not look alike. One image is enough to learn the pattern is bad
        // — it is the same pattern for all of them.
        if (!found.error().isEmpty()) {
            refused = found.error();
            break;
        }
        for (size_t i = 0; i < found.count(); ++i) {
            AdeMatch hit{};
            if (!found.at(i, &hit)) continue;
            const QString owner = ade::latin1(hit.owner);
            const QString region =
                QString::fromUtf8(ade_region_name(static_cast<AdeRegion>(hit.region)));

            auto *item = new QTreeWidgetItem(m_results);
            item->setText(0, QString::number(hit.offset));
            item->setText(1, owner.isEmpty() ? QStringLiteral("(%1)").arg(region)
                                             : QStringLiteral("%1  %2").arg(region, owner));
            item->setText(2, open->name);
            item->setTextAlignment(0, Qt::AlignRight | Qt::AlignVCenter);
            item->setData(0, RoleImage, static_cast<qulonglong>(open->index));
            item->setData(0, RolePartition, ADE_WHOLE_IMAGE);
            item->setData(0, RoleOffset, static_cast<qulonglong>(hit.offset));
            // A hit inside a file carries that file's block, so the row drags
            // out as the file — the same as a row found by name. A hit in the
            // bootblock or in unclaimed space carries none, and drags nothing,
            // because there is no file to give.
            if (hit.owner_block != 0) {
                item->setData(0, RoleBlock, hit.owner_block);
                item->setData(0, RoleIsDir, false);
            }
            item->setToolTip(1, item->text(1));
            item->setToolTip(2, open->path);
            ++hits;
        }
    }

    m_views->setCurrentWidget(m_results);
    if (!refused.isEmpty()) {
        m_results->clear();
        statusBar()->showMessage(QStringLiteral("Cannot search for \"%1\": %2").arg(query, refused));
        return;
    }
    statusBar()->showMessage(QStringLiteral("%1 match%2 for \"%3\" in %4 image%5")
                                 .arg(hits)
                                 .arg(hits == 1 ? "" : "es")
                                 .arg(query)
                                 .arg(m_images.size())
                                 .arg(m_images.size() == 1 ? "" : "s"));
}

void MainWindow::newDisk() {
    // The filesystems and their labels come from the engine, like every other
    // fact this window shows. A list written in Qt is a second opinion about
    // which disks exist, and the one most likely to drift.
    QDialog dialog(this);
    dialog.setWindowTitle(QStringLiteral("New disk"));

    auto *name = new QLineEdit(QStringLiteral("Empty"), &dialog);
    name->setMaxLength(30);  // what a rootblock's name field holds

    auto *type = new QComboBox(&dialog);
    type->setObjectName(QStringLiteral("newDiskType"));
    for (size_t i = 0; i < ade_create_type_count(); ++i) {
        type->addItem(QString::fromUtf8(ade_create_type_label(i)),
                      QString::fromUtf8(ade_create_type_name(i)));
    }
    // The default is the international FFS, because everything since Workbench
    // 2.0 writes it and a name with an accent sorts wrongly without it (C-006).
    type->setCurrentIndex(type->findData(QStringLiteral("ffs-intl")));

    auto *shape = new QComboBox(&dialog);
    shape->setObjectName(QStringLiteral("newDiskShape"));
    shape->addItem(QStringLiteral("3.5\" DD — 880 KB"), ADE_DISK_DD);
    shape->addItem(QStringLiteral("3.5\" HD — 1.76 MB"), ADE_DISK_HD);
    shape->addItem(QStringLiteral("5.25\" DD — 440 KB"), ADE_DISK_DD525);
    shape->addItem(QStringLiteral("Hard disk"), ADE_DISK_HARD);

    auto *megabytes = new QSpinBox(&dialog);
    // AmigaDOS addresses 4 GB, and `ade create` makes up to that; the window
    // stops at 1 GB because it holds a disk whole in memory and an edit
    // copies it, which at 4 GB is several times that in RAM (ROADMAP §Phase
    // 6, Scale). Past 49 MB the bitmap continues in a bm_ext chain, which
    // ADE writes as AmigaDOS's Format does.
    megabytes->setRange(1, 1024);
    megabytes->setObjectName(QStringLiteral("newDiskMegabytes"));
    megabytes->setToolTip(QStringLiteral(
        "Up to 1 GB here. The command line makes up to the 4 GB an AmigaDOS volume can\n"
        "address; the window holds a disk whole while it is edited, so it stops lower."));
    megabytes->setValue(8);
    megabytes->setSuffix(QStringLiteral(" MB"));
    megabytes->setEnabled(false);
    connect(shape, &QComboBox::currentIndexChanged, &dialog, [shape, megabytes] {
        megabytes->setEnabled(shape->currentData().toInt() == ADE_DISK_HARD);
    });

    auto *form = new QFormLayout;
    form->addRow(QStringLiteral("Volume name"), name);
    form->addRow(QStringLiteral("Filesystem"), type);
    form->addRow(QStringLiteral("Size"), shape);
    form->addRow(QStringLiteral("Hard disk size"), megabytes);

    auto *buttons =
        new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);

    auto *layout = new QVBoxLayout(&dialog);
    layout->addLayout(form);
    layout->addWidget(buttons);
    if (dialog.exec() != QDialog::Accepted) return;

    const int kind = shape->currentData().toInt();
    const QString suggested =
        QStringLiteral("%1.%2").arg(name->text(), kind == ADE_DISK_HARD ? "hdf" : "adf");
    const QString path = QFileDialog::getSaveFileName(
        this, QStringLiteral("Save the new disk as..."), QDir::homePath() + "/" + suggested);
    if (path.isEmpty()) return;

    const QByteArray typeName = type->currentData().toString().toUtf8();
    const QByteArray volumeName = name->text().toUtf8();
    const AdeResult made = ade_create(path.toLocal8Bit().constData(), typeName.constData(),
                                      volumeName.constData(),
                                      static_cast<AdeDiskShape>(kind),
                                      static_cast<uint32_t>(megabytes->value()));
    if (made != ADE_OK) {
        // "Already exists" is a thing the person can fix by choosing another
        // name, and it is worth saying rather than reporting a generic
        // failure — the file dialog will have asked about overwriting, and
        // ADE declines anyway, which needs explaining.
        emit errorOccurred(made == ADE_ALREADY_EXISTS
                               ? QStringLiteral("%1 already exists. ADE will not overwrite a "
                                                "disk image.")
                                     .arg(path)
                               : QStringLiteral("Could not create %1.").arg(path));
        return;
    }
    // Opened straight away, because a disk you cannot see is not obviously a
    // disk: the tree shows its volume name and the hex pane its bootblock.
    openCreated(path);
}

void MainWindow::extractAll() {
    const Open *open = imageFor(m_selected);
    if (!open) {
        emit errorOccurred(QStringLiteral("Select a disk first."));
        return;
    }
    const QString dir = QFileDialog::getExistingDirectory(
        this, QStringLiteral("Extract every file into..."), QDir::homePath());
    if (dir.isEmpty()) return;

    uint64_t written = 0;
    uint64_t skipped = 0;
    const quint32 partition = m_selected->data(0, RolePartition).isValid()
                                  ? m_selected->data(0, RolePartition).toUInt()
                                  : ADE_WHOLE_IMAGE;
    if (!open->image.unpack(partition, dir, &written, &skipped)) {
        emit errorOccurred(QStringLiteral("Could not extract %1 into %2.").arg(open->name, dir));
        return;
    }

    // A partial recovery is said plainly. "Extracted 40 files" when 3 were
    // skipped is how somebody comes to believe they have the whole disk.
    QString message = QStringLiteral("Extracted %1 file%2 from %3 into %4")
                          .arg(written)
                          .arg(written == 1 ? "" : "s")
                          .arg(open->name, dir);
    if (skipped > 0) {
        message += QStringLiteral("  —  %1 skipped, nothing was overwritten").arg(skipped);
    }
    statusBar()->showMessage(message);
}

void MainWindow::showSurface() {
    const Open *open = imageFor(m_selected);
    if (!open) {
        emit errorOccurred(QStringLiteral("Select a disk first."));
        return;
    }
    const QVector<AdeTrack> tracks = open->image.surface();
    if (tracks.isEmpty()) {
        // Enabled only when there is something to show, so this is a guard
        // rather than the ordinary path — but a plain ADF reaching here should
        // say why rather than open an empty grid.
        emit errorOccurred(
            QStringLiteral("%1 carries no track-level information: it is already "
                           "sectors, and nothing recorded how they were read.")
                .arg(open->name));
        return;
    }

    QDialog dialog(this);
    dialog.setWindowTitle(QStringLiteral("Disk surface — %1").arg(open->name));
    auto *view = new SurfaceView(&dialog);
    view->setTracks(tracks);

    auto *legend = new QLabel(&dialog);
    legend->setTextFormat(Qt::RichText);
    legend->setWordWrap(true);
    legend->setText(SurfaceView::legend(view->palette()));

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, &dialog);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);

    auto *layout = new QVBoxLayout(&dialog);
    layout->addWidget(view, 1);
    layout->addWidget(legend);
    layout->addWidget(buttons);
    dialog.resize(760, 320);
    dialog.exec();
}

void MainWindow::recoverLostFiles() {
    Open *open = imageFor(m_selected);
    if (!open) {
        emit errorOccurred(QStringLiteral("Select a disk first."));
        return;
    }

    const ade::Image::Carve carve = open->image.carve();
    const size_t count = carve.count();
    if (count == 0) {
        // Not an error, and said as a result rather than as a shrug: nothing
        // on this disk is outside its directory tree.
        emit errorOccurred(
            QStringLiteral("Nothing is lost on %1 — every block a file header "
                           "claims is reachable from the directory tree.")
                .arg(open->name));
        return;
    }

    QDialog dialog(this);
    dialog.setWindowTitle(QStringLiteral("Lost files — %1").arg(open->name));

    auto *table = new QTreeWidget(&dialog);
    table->setRootIsDecorated(false);
    table->setUniformRowHeights(true);
    table->setSelectionMode(QAbstractItemView::ExtendedSelection);
    table->setHeaderLabels({QStringLiteral("Block"), QStringLiteral("Name"),
                            QStringLiteral("Evidence"), QStringLiteral("Recoverable"),
                            QStringLiteral("Claimed")});

    size_t recoverable = 0;
    for (size_t i = 0; i < count; ++i) {
        const AdeEvidence how = carve.evidence(i);
        auto *row = new QTreeWidgetItem(table);
        row->setData(0, Qt::UserRole, static_cast<qulonglong>(i));
        // On a hard disk the block counts from its partition, which says so.
        const QString partition = carve.partition(i);
        row->setText(0, partition.isEmpty()
                            ? QString::number(carve.block(i))
                            : QStringLiteral("%1 %2").arg(partition).arg(carve.block(i)));
        row->setText(1, carve.name(i));
        row->setText(2, how == ADE_EVIDENCE_SELF_EVIDENT ? QStringLiteral("self-evident")
                        : how == ADE_EVIDENCE_PARTIAL    ? QStringLiteral("partial")
                                                         : QStringLiteral("header only"));
        row->setText(3, how == ADE_EVIDENCE_HEADER ? QStringLiteral("—")
                                                   : QString::number(carve.confirmed(i)));
        row->setText(4, QString::number(carve.size(i)));
        for (int column : {0, 3, 4}) row->setTextAlignment(column, Qt::AlignRight);
        // A header-only row is greyed, not hidden. It is real information —
        // a file was here, with this name and this size — and it is also the
        // one row nobody should treat as recovered content.
        if (how == ADE_EVIDENCE_HEADER) {
            for (int column = 0; column < table->columnCount(); ++column) {
                row->setForeground(column, dialog.palette().brush(QPalette::Disabled,
                                                                  QPalette::WindowText));
            }
        } else {
            ++recoverable;
        }
    }
    for (int column = 0; column < table->columnCount(); ++column) table->resizeColumnToContents(column);

    // Said in the dialog every time, because the grading is the whole reason
    // this feature could be built at all. Somebody reading the list has to
    // know that one of these three words does not mean "recovered".
    auto *legend = new QLabel(&dialog);
    legend->setWordWrap(true);
    legend->setTextFormat(Qt::RichText);
    legend->setText(QStringLiteral(
        "<b>self-evident</b> — every data block names this header back and checksums; "
        "the disk confirms the contents, not ADE.<br>"
        "<b>partial</b> — some blocks agree and some have been overwritten; what comes "
        "back is short, and is saved with a <tt>.partial</tt> suffix.<br>"
        "<b>header only</b> — the name and size are sound and <b>nothing confirms the "
        "contents</b>. FFS data blocks carry no header, so an FFS file can never be "
        "better than this. These are not written out.<br>"
        "<b>Put back on disk</b> undeletes into the folders they were in, as one step for "
        "Undo. A header-only file goes back with its name and size, its contents no better "
        "confirmed than here; anything whose blocks are in use again stays deleted, and "
        "says so."));

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, &dialog);
    QPushButton *recover = buttons->addButton(QStringLiteral("Recover to folder..."),
                                              QDialogButtonBox::ActionRole);
    recover->setEnabled(recoverable > 0);
    // F-012: back onto the disk itself, as an edit — where "Recover to
    // folder" copies out. Every row can be tried; what cannot go back says
    // why. A header-only file goes back with its name and size, and its
    // contents are as unconfirmed on the disk as they would be in a folder.
    QPushButton *putBack =
        buttons->addButton(QStringLiteral("Put back on disk"), QDialogButtonBox::ActionRole);
    putBack->setToolTip(QStringLiteral(
        "Undelete the selected entries — or all of them — into the folders they were in. "
        "One step for Undo; nothing is saved until you save the disk."));
    connect(putBack, &QPushButton::clicked, &dialog, [this, &dialog, &carve, table, count] {
        QVector<quint32> wanted;
        const QList<QTreeWidgetItem *> chosen = table->selectedItems();
        if (chosen.isEmpty()) {
            for (size_t i = 0; i < count; ++i) wanted.append(carve.block(i));
        } else {
            for (const QTreeWidgetItem *row : chosen) {
                wanted.append(carve.block(row->data(0, Qt::UserRole).toULongLong()));
            }
        }
        putBackDeleted(m_selected, wanted);
        // The list is the disk as it was: close it rather than show rows that
        // are no longer lost.
        dialog.accept();
    });
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);

    connect(recover, &QPushButton::clicked, &dialog, [this, &dialog, &carve, open, table, count] {
        const QString dir = QFileDialog::getExistingDirectory(
            &dialog, QStringLiteral("Recover lost files into..."), QDir::homePath());
        if (dir.isEmpty()) return;

        // A selection means "these"; no selection means all of them. The same
        // rule the extract items follow.
        QVector<size_t> wanted;
        const QList<QTreeWidgetItem *> chosen = table->selectedItems();
        if (chosen.isEmpty()) {
            for (size_t i = 0; i < count; ++i) wanted.append(i);
        } else {
            for (const QTreeWidgetItem *row : chosen) {
                if (row->columnCount() > 0 && !wanted.contains(row->data(0, Qt::UserRole).toULongLong()))
                    wanted.append(row->data(0, Qt::UserRole).toULongLong());
            }
        }

        size_t written = 0;
        size_t partial = 0;
        size_t unconfirmed = 0;
        size_t failed = 0;
        for (size_t i : wanted) {
            const AdeResult result = carve.write(open->image, i, dir);
            if (result == ADE_OK) {
                ++written;
                if (carve.evidence(i) == ADE_EVIDENCE_PARTIAL) ++partial;
            } else if (carve.evidence(i) == ADE_EVIDENCE_HEADER) {
                ++unconfirmed;
            } else {
                ++failed;
            }
        }

        QString message = QStringLiteral("Recovered %1 file%2 into %3")
                              .arg(written)
                              .arg(written == 1 ? QString() : QStringLiteral("s"), dir);
        // Every qualification is stated. "Recovered 12 files" while three came
        // back truncated is how somebody comes to trust a hole.
        if (partial > 0)
            message += QStringLiteral(", %1 incomplete and named .partial").arg(partial);
        if (unconfirmed > 0)
            message += QStringLiteral(", %1 not written because nothing confirms the contents")
                           .arg(unconfirmed);
        if (failed > 0) message += QStringLiteral(", %1 could not be written").arg(failed);
        emit errorOccurred(message);
    });

    auto *layout = new QVBoxLayout(&dialog);
    layout->addWidget(table, 1);
    layout->addWidget(legend);
    layout->addWidget(buttons);
    dialog.resize(680, 460);
    dialog.exec();
}

void MainWindow::showSpecs() {
    const Open *open = imageFor(m_selected);
    if (!open) {
        emit errorOccurred(QStringLiteral("Select a disk first."));
        return;
    }

    QString body = QStringLiteral("<h3>%1</h3>").arg(open->name.toHtmlEscaped());
    body += QStringLiteral("<p style='color:gray'>%1</p>").arg(describe(*open).toHtmlEscaped());

    // Every claim carries its evidence. A report that says "needs Kickstart
    // 2.0" without saying why is asking to be believed; one that names the
    // library is asking to be checked.
    body += QStringLiteral("<table cellpadding='3'>");
    for (const auto &[what, because] : open->image.specs()) {
        body += QStringLiteral("<tr><td valign='top'><b>%1</b></td>"
                               "<td valign='top' style='color:gray'>%2</td></tr>")
                    .arg(what.toHtmlEscaped(), because.toHtmlEscaped());
    }
    body += QStringLiteral("</table>");

    // And the blind spots, named. Left out, the silence reads as "there is
    // nothing more to know", and somebody concludes the disk runs on anything.
    body += QStringLiteral("<p><b>Not on the disk, and not inferable from it</b></p><table "
                           "cellpadding='3'>");
    for (size_t i = 0; i < ade_specs_unknowable_count(); ++i) {
        body += QStringLiteral("<tr><td valign='top'>%1</td>"
                               "<td valign='top' style='color:gray'>%2</td></tr>")
                    .arg(QString::fromUtf8(ade_specs_unknowable_what(i)).toHtmlEscaped(),
                         QString::fromUtf8(ade_specs_unknowable_why(i)).toHtmlEscaped());
    }
    body += QStringLiteral("</table>");

    QDialog dialog(this);
    dialog.setWindowTitle(QStringLiteral("Disk information — %1").arg(open->name));
    auto *text = new QTextBrowser(&dialog);
    text->setHtml(body);
    text->setOpenExternalLinks(false);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, &dialog);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    auto *layout = new QVBoxLayout(&dialog);
    layout->addWidget(text);
    layout->addWidget(buttons);
    dialog.resize(640, 460);
    dialog.exec();
}

void MainWindow::showAbout() {
    QMessageBox about(this);
    about.setWindowTitle(QStringLiteral("About Amiga Disk Engine"));
    about.setTextFormat(Qt::RichText);
    about.setIconPixmap(
        QIcon::fromTheme(QStringLiteral("drive-removable-media")).pixmap(64, 64));
    about.setText(aboutTitle());
    about.setInformativeText(aboutDetail());
    about.setStandardButtons(QMessageBox::Close);
    about.exec();
}

/// The map's own legend. Same shape as the hex pane's, different colours.
void MainWindow::showMapLegend(const QVector<HexRegion> &regions,
                               const ade::DiskDamage &damage) {
    if (!m_mapLegend) return;
    if (regions.isEmpty()) {
        m_mapLegend->hide();
        return;
    }
    QList<int> present;
    for (const HexRegion &span : regions) {
        if (!present.contains(span.region)) present.append(span.region);
    }
    std::sort(present.begin(), present.end());

    // Each swatch outlined, because the legend sits on the window background
    // while the swatches are mixed against the map's page, and `unclaimed`
    // is kept close to that page. The outline is a table cell's border: Qt's
    // rich text draws none on a span, which is how the first outline was
    // written and why it never showed.
    const QPalette legend = m_mapLegend->palette();
    const QColor text = legend.color(QPalette::WindowText);
    const QColor window = legend.color(QPalette::Window);
    const QColor frame = QColor::fromRgbF((text.redF() + window.redF()) / 2,
                                          (text.greenF() + window.greenF()) / 2,
                                          (text.blueF() + window.blueF()) / 2);
    QString row;
    QStringList tips;
    for (int region : present) {
        const QString name =
            QString::fromUtf8(ade_region_name(static_cast<AdeRegion>(region))).toHtmlEscaped();
        const QString describes =
            QString::fromUtf8(ade_region_describes(static_cast<AdeRegion>(region)))
                .toHtmlEscaped();
        row += QStringLiteral(
                   "<td><table border=1 cellspacing=0 cellpadding=0 "
                   "style='border-color:%1; border-style:solid'><tr>"
                   "<td bgcolor='%2' width=12 height=12></td></tr></table></td>"
                   "<td>&nbsp;%3&nbsp;&nbsp;&nbsp;&nbsp;</td>")
                   .arg(frame.name(), MapView::colourFor(m_map->palette(), region).name(), name);
        tips << QStringLiteral("<b>%1</b> — %2").arg(name, describes);
    }
    // The damage this disk has, after the regions, hatched as on the map.
    QList<int> kinds;
    for (const auto &[block, kind] : damage.blocks) {
        Q_UNUSED(block);
        if (!kinds.contains(kind)) kinds.append(kind);
    }
    std::sort(kinds.begin(), kinds.end());
    for (int kind : kinds) {
        const auto k = static_cast<AdeDamageKind>(kind);
        const QString name = QString::fromUtf8(ade_damage_name(k)).toHtmlEscaped();
        int count = 0;
        for (const auto &d : damage.blocks) count += d.second == kind ? 1 : 0;
        row += QStringLiteral(
                   "<td><table border=1 cellspacing=0 cellpadding=0 "
                   "style='border-color:%1; border-style:solid'><tr>"
                   "<td bgcolor='%2' width=12 height=12></td></tr></table></td>"
                   "<td>&nbsp;%3 (%4)&nbsp;&nbsp;&nbsp;&nbsp;</td>")
                   .arg(frame.name(), MapView::damageColourFor(m_map->palette(), kind).name(),
                        name)
                   .arg(count);
        tips << QStringLiteral("<b>%1</b> — %2")
                    .arg(name, QString::fromUtf8(ade_damage_describes(k)).toHtmlEscaped());
    }
    // And the counts, as the Atari engine's map gave them beside it.
    QString counts;
    if (damage.counts.known && damage.counts.blocks > 0) {
        const quint64 used = damage.counts.used;
        const quint64 all = damage.counts.blocks;
        counts = QStringLiteral("%1 of %2 blocks used (%3%), %4 free")
                     .arg(used)
                     .arg(all)
                     .arg(used * 100 / all)
                     .arg(damage.counts.free);
        if (damage.counts.files > 0) {
            counts += QStringLiteral("  ·  %1 file%2")
                          .arg(damage.counts.files)
                          .arg(damage.counts.files == 1 ? "" : "s");
            if (damage.counts.in_pieces > 0) {
                counts += QStringLiteral(", %1 in more than one piece").arg(damage.counts.in_pieces);
            }
        }
        if (damage.blocks.isEmpty()) counts += QStringLiteral("  ·  nothing damaged");
    }
    m_mapLegend->setText(
        QStringLiteral("<table cellspacing=0 cellpadding=0><tr>%1</tr></table>%2")
            .arg(row, counts.isEmpty() ? QString() : QStringLiteral("<p>%1</p>").arg(counts)));
    m_mapLegend->setToolTip(tips.join(QStringLiteral("<br>")));
    m_mapLegend->show();
}

/// Show the whole disk in hex, scrolled to `offset`.
void MainWindow::goToOffset(quint64 offset) {
    // Clicking a cell on the map is asking "what is in there", and the answer
    // is the bytes. The disk is already open, so this is the whole-disk view
    // scrolled to the block rather than a new read.
    if (m_diskRegions.isEmpty()) return;
    const int line = static_cast<int>(offset / hexview::BytesPerLine);
    m_views->setCurrentWidget(m_hexTab);
    m_hex->verticalScrollBar()->setValue(line);
    m_hex->selectBytesAt(offset);
    markWhatIsOnScreen();
}

void MainWindow::showLegend(const QVector<HexRegion> &regions) {
    if (!m_legend) return;
    if (regions.isEmpty()) {
        m_legend->hide();
        return;
    }
    // Only the regions this disk actually has. A legend listing a colour that
    // is nowhere on screen sends someone looking for it.
    QList<int> present;
    for (const HexRegion &span : regions) {
        if (!present.contains(span.region)) present.append(span.region);
    }
    std::sort(present.begin(), present.end());

    // One line: swatch and name only. The descriptions were tried inline and
    // took three lines out of the hex view to explain six colours, which is
    // the wrong trade in a pane whose whole job is showing bytes. They are the
    // tooltip instead — there when wanted, costing nothing when not.
    QStringList parts;
    QStringList tips;
    for (int region : present) {
        // Names and descriptions come from the engine, like every other fact
        // the window shows: the GUI knows nothing about Amiga filesystems, and
        // a legend written in Qt would be the first thing to drift from
        // `--format=json`.
        const QString name = QString::fromUtf8(ade_region_name(static_cast<AdeRegion>(region)))
                                 .toHtmlEscaped();
        const QString describes =
            QString::fromUtf8(ade_region_describes(static_cast<AdeRegion>(region)))
                .toHtmlEscaped();
        const QColor colour = HexPane::regionColour(m_hex->palette(), region);
        const QString swatch =
            colour.isValid()
                ? QStringLiteral("<span style='background:%1'>&nbsp;&nbsp;&nbsp;</span>")
                      .arg(colour.name())
                // A region with no tint still belongs in the legend, saying so:
                // "files are not coloured" is the answer to why most of the
                // disk is plain, and leaving it out makes that a mystery.
                : QStringLiteral("<span style='opacity:0.55'>&mdash;&mdash;&mdash;</span>");
        parts << QStringLiteral("%1&nbsp;%2").arg(swatch, name);
        tips << QStringLiteral("<b>%1</b> — %2").arg(name, describes);
    }
    m_legend->setText(parts.join(QStringLiteral("&nbsp;&nbsp;&nbsp;&nbsp;")));
    m_legend->setToolTip(tips.join(QStringLiteral("<br>")));
    m_legend->show();
}

void MainWindow::showHit(QTreeWidgetItem *item) {
    showWholeDisk(item);
    const quint64 offset = item->data(0, RoleOffset).toULongLong();
    // The hex tab first: its pane is filled only when it is showing
    // (IMP-009), and scrolling it before then scrolls an empty document.
    m_views->setCurrentWidget(m_hexTab);

    // A few lines above the hit rather than at the very top: a match on the
    // first visible line looks like the top of the view rather than like a
    // result, and the bytes before it are usually what makes it legible.
    constexpr int Lead = 3;
    const int line = static_cast<int>(offset / hexview::BytesPerLine);
    m_hex->verticalScrollBar()->setValue(qMax(0, line - Lead));
    // The hit itself, selected in the hex field so it is visible as well as
    // scrolled to. Two bytes is a placeholder for "here": the pane's selection
    // is per-field, and the search does not report the pattern's length.
    m_hex->selectBytesAt(offset);
    markWhatIsOnScreen();
}

void MainWindow::showWholeDisk(QTreeWidgetItem *item) {
    const Open *open = imageFor(item);
    if (!open) {
        clearViews();
        return;
    }
    const quint64 size = open->image.size();
    // The hex pane reads the disk as it is scrolled, a screenful at a time
    // (IMP-017), so it shows the whole of it however large. The reader holds
    // the image by pointer: `closeAll` clears the pane before the images go.
    const auto read = [open](quint64 offset, quint64 length) {
        const ade::Buffer buffer = open->image.readRange(offset, length);
        return buffer ? buffer.data() : QByteArray();
    };

    const QVector<HexRegion> regions = open->image.regions();
    buildMap(*open);
    m_hex->setSource(size, read);
    m_hex->setRegions(regions);
    m_diskRegions = regions;
    m_diskImage = item->data(0, RoleImage).toULongLong();
    m_diskPartition = item->data(0, RolePartition).toUInt();
    m_topLine = -1;
    if (m_follow) m_follow->start();
    // The text pane is filled when its tab is showing, not before (IMP-009):
    // on a whole disk it costs about 130 ms, spent on nothing if it is hidden.
    m_text->clear();
    m_pending = {true, {}, open};
    fillVisiblePreview();
    showLegend(regions);
    m_extract->setEnabled(false);

    QString message = describe(*open);
    if (size > HexPane::MaxBytes) {
        message += QStringLiteral("  —  hex shows the first %1 of %2 bytes")
                       .arg(HexPane::MaxBytes)
                       .arg(size);
    }
    statusBar()->showMessage(message);
}

QByteArray MainWindow::contentsOf(QTreeWidgetItem *item) const {
    if (!item || item->data(0, RoleIsDir).toBool()) return {};
    const Open *open = imageFor(item);
    if (!open) return {};
    const QVariant block = item->data(0, RoleBlock);
    if (!block.isValid()) return {};
    const ade::Buffer buffer =
        open->image.read(item->data(0, RolePartition).toUInt(), block.toUInt());
    if (!buffer) return {};
    return buffer.data();
}

void MainWindow::showEntry(QTreeWidgetItem *item) {
    m_selected = item;
    // The Assembly view follows the selection, but is only built when shown.
    m_asmStale = true;
    m_asmForceRaw = false;
    if (m_views && m_views->currentWidget() == m_asmTab) refreshAssembly();
    m_decodedStale = true;
    if (m_views && m_views->currentWidget() == m_decoded) refreshDecoded();
    // Extracting everything needs a disk, not a file: any row of any image
    // will do, because the whole disk is what it takes.
    if (m_extractAll) m_extractAll->setEnabled(imageFor(item) != nullptr);
    if (m_specs) m_specs->setEnabled(imageFor(item) != nullptr);
    // Enabled for any image, unlike the surface view below. "Nothing was
    // lost" is a result worth asking for and getting; the surface greys
    // because most containers *cannot know*, which is a different thing. It
    // is also not conditioned on the disk mounting: the disks worth carving
    // are largely the ones that do not.
    if (m_carve) m_carve->setEnabled(imageFor(item) != nullptr);
    // Offered only when the container actually recorded what came off the
    // medium. A greyed item on a plain ADF is the honest answer: most
    // containers cannot know, and a view of 160 whole tracks would be a
    // measurement nobody made.
    if (m_surface) {
        const Open *open = imageFor(item);
        m_surface->setEnabled(open != nullptr && !open->image.surface().isEmpty());
    }
    // A content-search hit is an offset, not a file. Selecting one shows the
    // whole disk and scrolls to it — a list of offsets you cannot go to is
    // half a feature, and going there is where F-022's colouring pays off:
    // the byte lands in a region the eye can already name.
    if (item && item->data(0, RoleOffset).isValid()) {
        showHit(item);
        return;
    }
    // An image row carries no block, and selecting one shows the **whole
    // disk**: the bytes a file view can never reach — the bootblock where
    // protection lives, the rootblock, the bitmap, and the space no directory
    // entry points at, which on a damaged disk is where the interesting part
    // is. Each of those is tinted, so the structure of the disk is visible
    // rather than having to be counted out in offsets.
    if (item && !item->data(0, RoleBlock).isValid()) {
        showWholeDisk(item);
        return;
    }
    if (!item || item->data(0, RoleIsDir).toBool()) {
        clearViews();
        // A folder is on a disk too: its map, with the folder's own block.
        if (item) {
            mapDiskOf(item);
            m_map->highlightOwner(item->data(0, RoleBlock).toUInt());
        }
        return;
    }
    m_extract->setEnabled(true);

    // Anything that is not the disk itself is one region, so there is nothing
    // to tint — and a leftover disk map would colour these bytes by where the
    // *disk's* offsets fell, which looks deliberate and is not.
    //
    // Done here, before the branches, because it must happen on every one of
    // them. It did not, and the empty-contents path below kept the previous
    // disk's colours and legend under the words "(no readable contents)".
    m_hex->clear();
    m_diskRegions.clear();
    if (m_follow) m_follow->stop();
    markRow(0);
    showLegend({});
    // The map belongs to the disk, not to the row. Selecting a file picks its
    // blocks out, which is the thing a listing cannot show — where the file
    // actually lives, and how scattered it is. On its own disk's map: built
    // now if a file is the first thing selected, or another disk's is up.
    mapDiskOf(item);
    m_map->highlightOwner(item->data(0, RoleBlock).toUInt());

    const QByteArray data = contentsOf(item);
    if (data.isEmpty()) {
        // Either unreadable or genuinely empty. Both are worth saying, and
        // neither is worth a dialog.
        m_pending = {};
        m_hex->setMessage(QStringLiteral("(no readable contents)"));
        m_text->clear();
        return;
    }
    const QByteArray head = data.left(PreviewBytes);

    // The hex pane paints only what is on screen, so it takes the bytes now;
    // the text pane waits until it is brought forward (IMP-009).
    m_hex->setBytes(head);
    m_text->clear();
    m_pending = {true, head, nullptr};
    fillVisiblePreview();
    if (data.size() > head.size()) {
        statusBar()->showMessage(
            QStringLiteral("Showing the first %1 of %2 bytes").arg(head.size()).arg(data.size()));
    } else {
        statusBar()->showMessage(QStringLiteral("%1 bytes").arg(data.size()));
    }
}

void MainWindow::mapDiskOf(QTreeWidgetItem *item) {
    const Open *open = imageFor(item);
    if (open && open->index != m_mapImage) buildMap(*open);
}

void MainWindow::buildMap(const Open &open) {
    const QVector<HexRegion> regions = open.image.regions();
    m_map->setMap(regions, open.image.size() / 512, 512);
    // What the health report finds wrong, block by block, over the regions.
    const ade::DiskDamage damage = open.image.damage();
    m_map->setDamage(damage.blocks);
    showMapLegend(regions, damage);
    m_mapImage = open.index;
}

void MainWindow::extractSelected() {
    if (!m_selected) return;
    const QByteArray data = contentsOf(m_selected);
    if (data.isEmpty()) {
        emit errorOccurred(QStringLiteral("This file could not be read."));
        return;
    }
    const QString target =
        QFileDialog::getSaveFileName(this, QStringLiteral("Extract to"), m_selected->text(0));
    if (target.isEmpty()) return;

    QFile out(target);
    if (!out.open(QIODevice::WriteOnly)) {
        emit errorOccurred(out.errorString());
        return;
    }
    out.write(data);
    statusBar()->showMessage(QStringLiteral("Wrote %1 bytes to %2").arg(data.size()).arg(target));
}

void MainWindow::clearViews() {
    m_pending = {};
    m_hex->clear();
    m_text->clear();
    // The map and its legend belong to whatever was being shown. Left behind,
    // they would explain colours that are no longer on screen — and worse, the
    // map would tint the next thing put in the pane by where *this* one's
    // offsets fell. Clearing the hex pane clears its regions.
    m_diskRegions.clear();
    if (m_follow) m_follow->stop();
    markRow(0);
    showLegend({});
    if (m_extract) m_extract->setEnabled(false);
    if (m_extractAll) m_extractAll->setEnabled(imageFor(m_selected) != nullptr);
}

void MainWindow::dragEnterEvent(QDragEnterEvent *event) {
    if (event->mimeData()->hasUrls()) event->acceptProposedAction();
}

void MainWindow::dropEvent(QDropEvent *event) {
    // Every dropped file is opened, not just the first: dropping a handful of
    // images is how a person sets up a cross-image search.
    for (const QUrl &url : event->mimeData()->urls()) {
        const QString path = url.toLocalFile();
        if (!path.isEmpty()) openImage(path);
    }
}

// ---------------------------------------------------------------------------
// Writing into a disk (D-018)
// ---------------------------------------------------------------------------

namespace {
// How deep a dropped folder is followed. A real tree is far shallower; the
// bound is for a link loop the host's own listing did not catch.
constexpr int MaxDropDepth = 32;
}  // namespace

MainWindow::Open *MainWindow::editTarget() {
    if (Open *open = imageFor(m_selected)) return open;
    if (m_lastEdited < m_images.size()) return m_images[m_lastEdited].get();
    if (m_images.size() == 1) return m_images.front().get();
    return nullptr;
}

quint32 MainWindow::directoryFor(const QTreeWidgetItem *item, const Open &open) const {
    if (!item) return 0;
    // A partition of a hard disk is not something ADE writes into: it edits
    // plain ADFs and hardfiles, whose file *is* the volume.
    if (item->data(0, RolePartition).toUInt() != ADE_WHOLE_IMAGE) return 0;
    if (!item->data(0, RoleIsDir).toBool()) return directoryFor(item->parent(), open);
    // The disk's own row carries no block; its directory is the root.
    if (!item->parent()) return open.image.rootBlock();
    return item->data(0, RoleBlock).toUInt();
}

void MainWindow::addPath(Open &open, quint32 dir, const QString &path, int depth, int *made,
                         QStringList *refused) {
    const QFileInfo info(path);
    const QString name = info.fileName();
    QByteArray amiga;
    if (!ade::amigaName(name, &amiga)) {
        refused->append(QStringLiteral("%1: an Amiga name is Latin-1, and this has a "
                                       "character outside it")
                            .arg(name));
        return;
    }
    // A link is not followed: it may lead out of what was dropped, or back
    // into it, and copying a disk's worth of somebody's home folder onto a
    // floppy because of one symlink is not what a drop means.
    if (info.isSymLink()) {
        refused->append(QStringLiteral("%1: a link, not copied").arg(name));
        return;
    }
    if (info.isDir()) {
        if (depth >= MaxDropDepth) {
            refused->append(QStringLiteral("%1: folders nested too deeply").arg(name));
            return;
        }
        const ade::EditResult r = open.image.makeDir(dir, amiga);
        if (!r.ok()) {
            refused->append(QStringLiteral("%1: %2").arg(name, r.message));
            return;
        }
        ++*made;
        const QFileInfoList children = QDir(path).entryInfoList(
            QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden | QDir::System,
            QDir::Name);
        for (const QFileInfo &child : children) {
            addPath(open, r.block, child.filePath(), depth + 1, made, refused);
        }
        return;
    }
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        refused->append(QStringLiteral("%1: %2").arg(name, file.errorString()));
        return;
    }
    const ade::EditResult r = open.image.addFile(dir, amiga, file.readAll());
    if (!r.ok()) {
        refused->append(QStringLiteral("%1: %2").arg(name, r.message));
        return;
    }
    ++*made;
}

bool MainWindow::mayOverwriteFreeData(Open &open, const QString &doing) {
    const size_t held = open.image.freeBlocksHoldingData();
    if (held == 0 || open.freeDataAllowed) return true;
    // Asked rather than refused: reusing an old game disk as a blank is a
    // fair thing to do, and the person is the one who knows which this is.
    if (!m_confirmFreeData ||
        !m_confirmFreeData(
            QStringLiteral("%1 holds no files, but %2 block%3 its bitmap calls free hold%4 "
                           "data. On a disk like this that is usually a game its own loader "
                           "reads, and %5 would overwrite it.\n\nWrite on it anyway? Undo "
                           "brings it back until you save the disk.")
                .arg(open.name)
                .arg(held)
                .arg(held == 1 ? "" : "s")
                .arg(held == 1 ? "s" : "")
                .arg(doing))) {
        return false;
    }
    open.image.allowOverwritingFreeData();
    open.freeDataAllowed = true;
    return true;
}

int MainWindow::addFiles(QTreeWidgetItem *target, const QStringList &paths) {
    Open *open = imageFor(target);
    if (!open) return 0;
    const quint32 dir = directoryFor(target, *open);
    if (dir == 0) {
        emit errorOccurred(QStringLiteral(
            "ADE copies files onto plain ADF and hardfile images only, not into a "
            "partition of a hard disk."));
        return 0;
    }
    if (!mayOverwriteFreeData(*open, QStringLiteral("copying files there"))) return 0;
    int made = 0;
    QStringList refused;
    // One drop is one action, so one Undo takes all of it back (IMP-014).
    open->image.beginGroup();
    for (const QString &path : paths) addPath(*open, dir, path, 0, &made, &refused);
    open->image.endGroup();
    m_lastEdited = open->index;
    if (made > 0) reloadImage(*open);
    if (!refused.isEmpty()) {
        emit errorOccurred(QStringLiteral("Not everything could be copied onto %1:\n\n%2")
                               .arg(open->name, refused.join(QLatin1Char('\n'))));
    }
    if (made > 0) {
        statusBar()->showMessage(
            QStringLiteral("Copied %1 item%2 onto %3 — not saved yet; File ▸ Save as… keeps it")
                .arg(made)
                .arg(made == 1 ? "" : "s")
                .arg(open->name));
    }
    return made;
}

bool MainWindow::makeFolder(QTreeWidgetItem *target, const QString &name) {
    Open *open = imageFor(target);
    if (!open) return false;
    const quint32 dir = directoryFor(target, *open);
    if (dir == 0) {
        emit errorOccurred(QStringLiteral(
            "ADE makes folders on plain ADF and hardfile images only, not in a partition "
            "of a hard disk."));
        return false;
    }
    QByteArray amiga;
    if (!ade::amigaName(name, &amiga)) {
        emit errorOccurred(QStringLiteral("%1: an Amiga name is Latin-1, and this has a "
                                          "character outside it.")
                               .arg(name));
        return false;
    }
    if (!mayOverwriteFreeData(*open, QStringLiteral("making a folder"))) return false;
    const ade::EditResult r = open->image.makeDir(dir, amiga);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not make %1: %2").arg(name, r.message));
        return false;
    }
    reloadImage(*open);
    statusBar()->showMessage(
        QStringLiteral("Made %1 on %2 — not saved yet").arg(name, open->name));
    return true;
}

// Whether `item` is an entry ADE can delete or rename: a row inside a plain
// image's tree, not a disk's own row, a search hit or a row in a partition.
static bool isEditableEntry(const QTreeWidgetItem *item, const QTreeWidget *tree) {
    return item && item->treeWidget() == tree && item->parent() &&
           item->data(0, RoleBlock).isValid() &&
           item->data(0, RolePartition).toUInt() == ADE_WHOLE_IMAGE;
}

bool MainWindow::deleteItem(QTreeWidgetItem *item) {
    Open *open = imageFor(item);
    if (!open || !isEditableEntry(item, m_tree)) return false;
    const QString name = item->text(ColName);
    const bool folder = item->data(0, RoleIsDir).toBool();
    // A folder is deleted with what is in it. Asked first only when it holds
    // something: a file, or an empty folder, comes back with Undo.
    if (folder) {
        const ade::Listing inside =
            open->image.list(ADE_WHOLE_IMAGE, item->data(0, RoleBlock).toUInt());
        if (inside && inside.count() > 0 && m_confirmDelete &&
            !m_confirmDelete(QStringLiteral("Delete the folder %1 and everything in it?\n\n"
                                            "Undo brings it back until you save the disk.")
                                 .arg(name))) {
            return false;
        }
    }
    const ade::EditResult r =
        open->image.deleteEntry(item->data(0, RoleBlock).toUInt(), folder);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not delete %1: %2").arg(name, r.message));
        return false;
    }
    reloadImage(*open);
    statusBar()->showMessage(
        QStringLiteral("Deleted %1 from %2 — not saved yet").arg(name, open->name));
    return true;
}

bool MainWindow::makeLinkTo(QTreeWidgetItem *item, const QString &name) {
    Open *open = imageFor(item);
    if (!open || !isEditableEntry(item, m_tree)) return false;
    // Beside the entry: in the folder that holds it, which for an entry at
    // the top of the disk is the disk's own row.
    const quint32 dir = directoryFor(item->parent(), *open);
    if (dir == 0) return false;
    QByteArray amiga;
    if (!ade::amigaName(name, &amiga)) {
        emit errorOccurred(QStringLiteral("%1: an Amiga name is Latin-1, and this has a "
                                          "character outside it.")
                               .arg(name));
        return false;
    }
    if (!mayOverwriteFreeData(*open, QStringLiteral("making a link"))) return false;
    const QString target = item->text(ColName);
    const ade::EditResult r =
        open->image.makeLink(item->data(0, RoleBlock).toUInt(), dir, amiga);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not link %1 to %2: %3").arg(name, target, r.message));
        return false;
    }
    reloadImage(*open);
    statusBar()->showMessage(
        QStringLiteral("Linked %1 to %2 — not saved yet").arg(name, target));
    return true;
}

QString MainWindow::putBackDeleted(QTreeWidgetItem *item, const QVector<quint32> &blocks) {
    Open *open = imageFor(item);
    if (!open || blocks.isEmpty()) return QString();
    QHash<quint32, QString> names;
    {
        const ade::Image::Carve carve = open->image.carve();
        for (size_t i = 0; i < carve.count(); ++i) names.insert(carve.block(i), carve.name(i));
    }
    const auto nameOf = [&names](quint32 block) {
        return names.value(block, QStringLiteral("block %1").arg(block));
    };

    // One Undo takes the lot off again. Retried until a pass puts nothing
    // back: a file's folder may be later in the list than the file.
    QVector<quint32> left = blocks;
    QHash<quint32, QString> why;
    QStringList back;
    open->image.beginGroup();
    for (bool progress = true; progress && !left.isEmpty();) {
        progress = false;
        for (int i = 0; i < left.size();) {
            const ade::EditResult r = open->image.undeleteEntry(left.at(i));
            if (r.ok()) {
                back.append(nameOf(left.at(i)));
                why.remove(left.at(i));
                left.removeAt(i);
                progress = true;
            } else {
                why.insert(left.at(i), r.message);
                ++i;
            }
        }
    }
    open->image.endGroup();
    m_lastEdited = open->index;
    if (!back.isEmpty()) reloadImage(*open);

    QString message = back.isEmpty()
                          ? QStringLiteral("Nothing was put back on %1").arg(open->name)
                          : QStringLiteral("Put back %1 on %2 — not saved yet")
                                .arg(back.join(QStringLiteral(", ")), open->name);
    // Each refusal with its reason: "3 stayed deleted" alone would leave
    // somebody guessing which, and why.
    for (quint32 block : std::as_const(left)) {
        message += QStringLiteral("\n%1 stayed deleted: %2").arg(nameOf(block), why.value(block));
    }
    if (left.isEmpty()) {
        statusBar()->showMessage(message);
    } else {
        emit errorOccurred(message);
    }
    return message;
}

bool MainWindow::renameItem(QTreeWidgetItem *item, const QString &name) {
    Open *open = imageFor(item);
    if (!open || !isEditableEntry(item, m_tree)) return false;
    QByteArray amiga;
    if (!ade::amigaName(name, &amiga)) {
        emit errorOccurred(QStringLiteral("%1: an Amiga name is Latin-1, and this has a "
                                          "character outside it.")
                               .arg(name));
        return false;
    }
    const QString old = item->text(ColName);
    const ade::EditResult r = open->image.renameEntry(item->data(0, RoleBlock).toUInt(), amiga);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not rename %1: %2").arg(old, r.message));
        return false;
    }
    reloadImage(*open);
    statusBar()->showMessage(
        QStringLiteral("Renamed %1 to %2 — not saved yet").arg(old, name));
    return true;
}

QString MainWindow::repairDisk(QTreeWidgetItem *item) {
    Open *open = item ? imageFor(item) : editTarget();
    if (!open) return {};
    const ade::EditResult r = open->image.repair(false);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not repair %1: %2").arg(open->name, r.message));
        return {};
    }
    reloadImage(*open);
    if (open->image.isModified()) {
        statusBar()->showMessage(QStringLiteral("Repaired %1 — not saved yet").arg(open->name));
    }
    emit reported(QStringLiteral("Repair — %1").arg(open->name), r.message);
    return r.message;
}

void MainWindow::repairSelected() { repairDisk(m_selected); }

int MainWindow::moveItems(QTreeWidgetItem *target, const QList<QTreeWidgetItem *> &rows) {
    Open *open = imageFor(target);
    if (!open || rows.isEmpty()) return 0;
    const quint32 to = directoryFor(target, *open);
    if (to == 0) {
        emit errorOccurred(QStringLiteral(
            "ADE moves entries on plain ADF and hardfile images only, not in a partition "
            "of a hard disk."));
        return 0;
    }
    // Blocks and names first: the rows are rebuilt after the first move.
    QList<QPair<quint32, QString>> moving;
    QStringList refused;
    for (QTreeWidgetItem *row : rows) {
        if (!isEditableEntry(row, m_tree)) continue;
        if (imageFor(row) != open) {
            refused.append(QStringLiteral("%1: on another disk; drag it out and in to copy it")
                               .arg(row->text(ColName)));
            continue;
        }
        // Dropped on itself — let go a pixel from where the drag began — or
        // where it already is: nothing to do, and nothing to report.
        if (row == target || directoryFor(row->parent(), *open) == to) continue;
        moving.append({row->data(0, RoleBlock).toUInt(), row->text(ColName)});
    }
    int moved = 0;
    open->image.beginGroup();
    for (const auto &[block, name] : moving) {
        const ade::EditResult r = open->image.moveEntry(block, to);
        if (r.ok()) {
            ++moved;
        } else {
            refused.append(QStringLiteral("%1: %2").arg(name, r.message));
        }
    }
    open->image.endGroup();
    m_lastEdited = open->index;
    if (moved > 0) reloadImage(*open);
    if (!refused.isEmpty()) {
        emit errorOccurred(QStringLiteral("Not everything could be moved:\n\n%1")
                               .arg(refused.join(QLatin1Char('\n'))));
    }
    if (moved > 0) {
        statusBar()->showMessage(QStringLiteral("Moved %1 item%2 on %3 — not saved yet")
                                     .arg(moved)
                                     .arg(moved == 1 ? "" : "s")
                                     .arg(open->name));
    }
    return moved;
}

bool MainWindow::formatDisk(QTreeWidgetItem *item, const QString &name) {
    Open *open = item ? imageFor(item) : editTarget();
    if (!open) return false;
    QByteArray amiga;
    if (!name.isEmpty() && !ade::amigaName(name, &amiga)) {
        emit errorOccurred(QStringLiteral("%1: an Amiga name is Latin-1, and this has a "
                                          "character outside it.")
                               .arg(name));
        return false;
    }
    const ade::EditResult r = open->image.format(amiga);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not format %1: %2").arg(open->name, r.message));
        return false;
    }
    reloadImage(*open);
    statusBar()->showMessage(QStringLiteral("Formatted %1 — not saved yet; Undo brings "
                                            "everything back")
                                 .arg(open->name));
    return true;
}

QString MainWindow::wipeDisk(QTreeWidgetItem *item, bool freeBlocks) {
    Open *open = item ? imageFor(item) : editTarget();
    if (!open) return {};
    // A disk whose free blocks hold a game is asked about in those terms;
    // the general question would only be the same answer asked twice.
    if (freeBlocks && open->image.freeBlocksHoldingData() > 0) {
        if (!mayOverwriteFreeData(*open, QStringLiteral("wiping free space"))) return {};
    } else if (freeBlocks && m_confirmWipe &&
        !m_confirmWipe(QStringLiteral("Zero every free block of %1, and each file's slack?\n\n"
                                      "Nothing deleted from it can be recovered after. A game "
                                      "that reads blocks directly may keep data in blocks its "
                                      "bitmap calls free, and would lose it.\n\nUndo brings "
                                      "it back until you save the disk.")
                           .arg(open->name))) {
        return {};
    }
    const ade::EditResult r = open->image.wipe(true, freeBlocks);
    m_lastEdited = open->index;
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not wipe %1: %2").arg(open->name, r.message));
        return {};
    }
    reloadImage(*open);
    emit reported(QStringLiteral("Wipe — %1").arg(open->name), r.message);
    return r.message;
}

void MainWindow::formatSelected() {
    Open *open = imageFor(m_selected);
    if (!open) return;
    bool ok = false;
    const QString name = QInputDialog::getText(
        this, QStringLiteral("Format %1").arg(open->name),
        QStringLiteral("Everything on it leaves its folders; Undo brings it back until you "
                       "save.\n\nVolume name (empty keeps the one it has):"),
        QLineEdit::Normal, QString(), &ok);
    if (!ok) return;
    formatDisk(m_selected, name);
}

void MainWindow::wipeSlackSelected() { wipeDisk(m_selected, false); }

void MainWindow::wipeFreeSelected() { wipeDisk(m_selected, true); }

void MainWindow::deleteSelected() { deleteItem(m_selected); }

void MainWindow::renameSelected() {
    QTreeWidgetItem *item = m_selected;
    if (!isEditableEntry(item, m_tree)) return;
    bool ok = false;
    const QString name =
        QInputDialog::getText(this, QStringLiteral("Rename"), QStringLiteral("New name"),
                              QLineEdit::Normal, item->text(ColName), &ok);
    if (!ok || name.isEmpty() || name == item->text(ColName)) return;
    renameItem(item, name);
}

bool MainWindow::saveImageAs(QTreeWidgetItem *item, const QString &path) {
    Open *open = item ? imageFor(item) : editTarget();
    if (!open || path.isEmpty()) return false;
    const ade::EditResult r = open->image.saveAs(path);
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not save %1: %2").arg(path, r.message));
        return false;
    }
    reloadImage(*open);
    statusBar()->showMessage(QStringLiteral("Saved %1").arg(path));
    return true;
}

void MainWindow::save() { saveImage(nullptr); }

void MainWindow::openCreated(const QString &path) {
    const size_t before = m_images.size();
    openImage(path);
    if (m_images.size() > before) m_images.back()->createdHere = true;
}

bool MainWindow::saveImage(QTreeWidgetItem *item) {
    Open *open = item ? imageFor(item) : editTarget();
    if (!open || !open->image.isModified()) return false;

    // After a Save As, Save goes to that file: ADE made it in this session.
    if (!open->image.saveDestination().isEmpty()) {
        const ade::EditResult r = open->image.save();
        if (!r.ok()) {
            emit errorOccurred(QStringLiteral("Could not save: %1").arg(r.message));
            return false;
        }
        reloadImage(*open);
        statusBar()->showMessage(QStringLiteral("Saved %1").arg(open->image.saveDestination()));
        return true;
    }

    // Otherwise over the file it was opened from (D-020): asked first, and the
    // original kept the first time. A disk this window made is ADE's own file,
    // so it is neither asked about nor backed up.
    if (!open->createdHere) {
        const QString question =
            open->backup.isEmpty()
                ? QStringLiteral("Save the changes over \u201c%1\u201d?\n\nThe disk as it was "
                                 "when you opened it will be kept beside it as %2.orig.")
                      .arg(open->name, open->name)
                : QStringLiteral("Save the changes over \u201c%1\u201d?\n\nThe original is "
                                 "already kept as %2.")
                      .arg(open->name, QFileInfo(open->backup).fileName());
        if (!m_confirmSave || !m_confirmSave(question)) return false;
    }
    QString backup;
    const ade::EditResult r = open->image.saveOver(!open->createdHere, &backup);
    if (!r.ok()) {
        emit errorOccurred(QStringLiteral("Could not save %1: %2").arg(open->name, r.message));
        return false;
    }
    if (!backup.isEmpty()) open->backup = backup;
    reloadImage(*open);
    statusBar()->showMessage(
        backup.isEmpty()
            ? QStringLiteral("Saved %1").arg(open->path)
            : QStringLiteral("Saved %1 \u2014 the original is kept as %2")
                  .arg(open->path, QFileInfo(backup).fileName()));
    return true;
}

void MainWindow::saveAs() {
    Open *open = editTarget();
    if (!open) return;
    const QFileInfo from(open->path);
    const QString suggested =
        open->image.saveDestination().isEmpty()
            ? from.dir().filePath(
                  QStringLiteral("%1-edited.%2").arg(from.completeBaseName(), from.suffix()))
            : open->image.saveDestination();
    // No overwrite prompt from the dialog: ADE declines to overwrite anyway,
    // and a "Replace?" it then refuses would be the dialog promising something
    // the engine will not do.
    const QString path = QFileDialog::getSaveFileName(
        this, QStringLiteral("Save %1 as a new image").arg(open->name), suggested,
        QStringLiteral("Disk images (*.adf *.hdf);;All files (*)"), nullptr,
        QFileDialog::DontConfirmOverwrite);
    if (path.isEmpty()) return;
    saveImageAs(nullptr, path);
}

void MainWindow::newFolder() {
    QTreeWidgetItem *target = m_selected;
    if (!imageFor(target) && m_tree->topLevelItemCount() == 1) target = m_tree->topLevelItem(0);
    if (!imageFor(target)) return;
    bool ok = false;
    const QString name = QInputDialog::getText(this, QStringLiteral("New folder"),
                                               QStringLiteral("Name"), QLineEdit::Normal,
                                               QStringLiteral("New folder"), &ok);
    if (!ok || name.isEmpty()) return;
    makeFolder(target, name);
}

bool MainWindow::undoEdit() {
    Open *open = editTarget();
    if (!open || !open->image.undo()) return false;
    m_lastEdited = open->index;
    reloadImage(*open);
    statusBar()->showMessage(QStringLiteral("Undone on %1").arg(open->name));
    return true;
}

bool MainWindow::redoEdit() {
    Open *open = editTarget();
    if (!open || !open->image.redo()) return false;
    m_lastEdited = open->index;
    reloadImage(*open);
    statusBar()->showMessage(QStringLiteral("Redone on %1").arg(open->name));
    return true;
}

void MainWindow::reloadImage(Open &open) {
    QTreeWidgetItem *root = nullptr;
    for (int i = 0; i < m_tree->topLevelItemCount(); ++i) {
        QTreeWidgetItem *item = m_tree->topLevelItem(i);
        if (item->data(0, RoleImage).toULongLong() == open.index) root = item;
    }
    if (!root) return;

    // Which folders were open, by block, so the tree comes back as it was.
    // Blocks rather than names: a name can be added twice in two folders, a
    // block is one place.
    std::set<quint32> expanded;
    for (QTreeWidgetItemIterator it(root); *it; ++it) {
        if ((*it)->isExpanded() && (*it)->data(0, RoleIsDir).toBool()) {
            expanded.insert((*it)->data(0, RoleBlock).toUInt());
        }
    }

    // Everything that points into the rows about to go, let go first. The
    // views, the marked row and the search results all hold item pointers or
    // block numbers that an undo can make mean nothing.
    // A search result is about to be deleted whichever disk it belongs to,
    // so a selected one is let go as well as a row of this disk.
    const bool ownsSelection =
        imageFor(m_selected) == &open || (m_selected && m_selected->treeWidget() == m_results);
    if (ownsSelection) {
        m_syncing = true;
        m_tree->clearSelection();
        m_results->clearSelection();
        m_syncing = false;
        m_selected = nullptr;
        clearViews();
    }
    m_results->clear();
    qDeleteAll(root->takeChildren());

    root->setText(ColName, QStringLiteral("%1   %2").arg(open.name, describe(open)));
    root->setToolTip(ColName, rootToolTip(open));
    if (open.image.hasVolume()) {
        populate(root, open, ADE_WHOLE_IMAGE, open.image.rootBlock());
        addKindRow(root, open);
        root->setExpanded(true);
        // Breadth first: expanding a row populates it, and only then are its
        // own children there to be expanded.
        QList<QTreeWidgetItem *> pending{root};
        while (!pending.isEmpty()) {
            QTreeWidgetItem *item = pending.takeFirst();
            for (int i = 0; i < item->childCount(); ++i) {
                QTreeWidgetItem *child = item->child(i);
                if (child->data(0, RoleIsDir).toBool() &&
                    expanded.count(child->data(0, RoleBlock).toUInt()) > 0) {
                    child->setExpanded(true);
                    pending.append(child);
                }
            }
        }
    }
    // An edit moves blocks: a map of this disk is drawn again.
    if (m_mapImage == open.index) buildMap(open);
    showSummary();
    updateEditActions();
}

void MainWindow::updateEditActions() {
    Open *open = editTarget();
    const bool plain = open && open->image.hasVolume() &&
                       (!m_selected || m_selected->data(0, RolePartition).toUInt() ==
                                           ADE_WHOLE_IMAGE);
    if (m_newFolder) m_newFolder->setEnabled(plain);
    if (m_repair) m_repair->setEnabled(plain);
    if (m_format) m_format->setEnabled(plain);
    if (m_wipeSlack) m_wipeSlack->setEnabled(plain);
    if (m_wipeFree) m_wipeFree->setEnabled(plain);
    const bool entry = plain && isEditableEntry(m_selected, m_tree);
    if (m_delete) m_delete->setEnabled(entry);
    if (m_rename) m_rename->setEnabled(entry);
    if (m_makeLink) m_makeLink->setEnabled(entry);
    if (m_undo) m_undo->setEnabled(open && open->image.canUndo());
    if (m_redo) m_redo->setEnabled(open && open->image.canRedo());
    if (m_save) m_save->setEnabled(open && open->image.isModified());
    // Save As stays offered after a save, for a second copy — but only for a
    // disk that has been edited, since there is nothing else to save.
    if (m_saveAs) {
        m_saveAs->setEnabled(open && (open->image.isModified() ||
                                      !open->image.saveDestination().isEmpty()));
    }
}

bool MainWindow::mayDiscardEdits() {
    QStringList unsaved;
    for (const auto &open : m_images) {
        if (open->image.isModified()) unsaved << open->name;
    }
    if (unsaved.isEmpty()) return true;
    if (!m_confirmDiscard) return false;
    return m_confirmDiscard(
        QStringLiteral("%1 unsaved changes. Close anyway and lose them?\n\n%2")
            .arg(unsaved.size() == 1 ? QStringLiteral("One disk has")
                                     : QStringLiteral("%1 disks have").arg(unsaved.size()),
                 unsaved.join(QLatin1Char('\n'))));
}

void MainWindow::closeEvent(QCloseEvent *event) {
    if (mayDiscardEdits()) {
        event->accept();
    } else {
        event->ignore();
    }
}

void MainWindow::filesDropped(QTreeWidgetItem *target, const QList<QUrl> &urls) {
    QStringList paths;
    for (const QUrl &url : urls) {
        if (url.isLocalFile()) paths << url.toLocalFile();
    }
    // Onto a disk: copy onto it. Onto empty space: open as disks, which is
    // what dropping onto the window has always meant.
    if (target && imageFor(target)) {
        addFiles(target, paths);
        return;
    }
    for (const QString &path : paths) openImage(path);
}

// ---------------------------------------------------------------------------
// The Decoded view (F-033)
// ---------------------------------------------------------------------------

namespace {

// The picture as crisp pixel art over a checkerboard, so its transparent
// pixels read as transparent rather than as the page.
// Each Amiga pixel drawn as a block, as large as fits up to 8 times, in the
// shape the Amiga showed it: a Workbench icon's pixels are twice as tall as
// they are wide, and drawn square it looks flattened. A 16-pixel pointer is
// drawn 8 times over; a 320-pixel picture at most twice.
QImage pixelArt(const QImage &picture, const QPalette &palette, QSize shape) {
    constexpr int MaxScale = 8;
    constexpr int MaxWidth = 640;
    constexpr int MaxHeight = 512;
    const int wide = qMax(1, shape.width());
    const int tall = qMax(1, shape.height());
    const int s = qBound(1, qMin(MaxWidth / qMax(1, picture.width() * wide),
                                 MaxHeight / qMax(1, picture.height() * tall)),
                         MaxScale);
    const int sx = s * wide;
    const int sy = s * tall;
    QImage out(picture.width() * sx, picture.height() * sy, QImage::Format_ARGB32);
    const QColor base = palette.color(QPalette::Base);
    const QColor shade = base.lightness() > 128 ? base.darker(112) : base.lighter(135);
    const int cell = qMax(1, sx / 2);
    for (int y = 0; y < out.height(); ++y) {
        for (int x = 0; x < out.width(); ++x) {
            const QRgb px = picture.pixel(x / sx, y / sy);
            const bool check = ((x / cell) + (y / cell)) % 2 != 0;
            out.setPixel(x, y, qAlpha(px) ? px : (check ? shade : base).rgb());
        }
    }
    return out;
}

// A colour as a framed swatch: a span's background has no border, and a
// white colour on a white page would be no swatch at all.
QImage swatch(QRgb colour, const QPalette &palette) {
    QImage out(18, 12, QImage::Format_RGB32);
    out.fill(colour);
    const QRgb frame = palette.color(QPalette::Text).rgb();
    for (int x = 0; x < out.width(); ++x) {
        out.setPixel(x, 0, frame);
        out.setPixel(x, out.height() - 1, frame);
    }
    for (int y = 0; y < out.height(); ++y) {
        out.setPixel(0, y, frame);
        out.setPixel(out.width() - 1, y, frame);
    }
    return out;
}

}  // namespace

void MainWindow::refreshDecoded() {
    if (!m_decodedStale || !m_decoded) return;
    m_decodedStale = false;
    const auto say = [this](const QString &why) {
        m_decoded->setHtml(QStringLiteral("<p>%1</p>").arg(why.toHtmlEscaped()));
    };

    QTreeWidgetItem *item = m_selected;
    const Open *open = imageFor(item);
    if (!open || !item->parent() || item->data(0, RoleOffset).isValid() ||
        item->data(0, RoleIsDir).toBool() || !item->data(0, RoleBlock).isValid()) {
        say(QStringLiteral("Select a file to see what it says. ADE decodes Intuition's "
                           "Preferences (devs/system-configuration)."));
        return;
    }
    const ade::Decoded decoded = open->image.decode(item->data(0, RolePartition).toUInt(),
                                                    item->data(0, RoleBlock).toUInt());
    const QString name = item->text(ColName);
    if (!decoded.read) {
        say(QStringLiteral("%1 could not be read.").arg(name));
        return;
    }
    if (!decoded.refusal.isEmpty()) {
        say(QStringLiteral("%1 is %2").arg(name, decoded.refusal));
        return;
    }

    // Laid out by the engine's groups, in its order: the GUI adds the
    // swatches and the picture, and decides nothing about the format. One
    // table, so the values line up from group to group.
    QString html = QStringLiteral("<h3>%1</h3>").arg(decoded.title.toHtmlEscaped());
    if (!decoded.picture.isNull()) {
        const QUrl url(QStringLiteral("ade-decoded:picture"));
        m_decoded->document()->addResource(QTextDocument::ImageResource, url,
                                           pixelArt(decoded.picture, m_decoded->palette(), decoded.pixelShape));
        html += QStringLiteral("<p><img src='%1'><br>%2</p>")
                    .arg(url.toString(), decoded.caption.toHtmlEscaped());
    }
    html += QStringLiteral("<table cellspacing='0' cellpadding='2'>");
    QString group;
    for (const ade::DecodedField &field : decoded.fields) {
        if (field.group != group) {
            group = field.group;
            html += QStringLiteral("<tr><td colspan='2' style='padding-top:10px'><b>%1</b>"
                                   "</td></tr>")
                        .arg(group.toHtmlEscaped());
        }
        QString value = field.value.toHtmlEscaped();
        if (field.colour >= 0) {
            const QString hex = QStringLiteral("%1").arg(field.colour, 6, 16, QLatin1Char('0'));
            const QUrl url(QStringLiteral("ade-decoded:swatch/") + hex);
            m_decoded->document()->addResource(
                QTextDocument::ImageResource, url,
                swatch(0xFF000000u | static_cast<QRgb>(field.colour), m_decoded->palette()));
            value = QStringLiteral("<img src='%1' title='#%2'>&nbsp;%3")
                        .arg(url.toString(), hex, value);
        }
        html += QStringLiteral("<tr><td style='padding-right:16px'>%1</td><td>%2</td></tr>")
                    .arg(field.label.toHtmlEscaped(), value);
    }
    html += QStringLiteral("</table>");
    m_decoded->setHtml(html);
}

// ---------------------------------------------------------------------------
// The Assembly view (F-032)
// ---------------------------------------------------------------------------

ade::Disassembly MainWindow::assemblyFor(int view, QString &what, QString &why) const {
    QTreeWidgetItem *item = m_selected;
    const Open *open = imageFor(item);
    if (!open) {
        why = QStringLiteral("Select a program to see its 68000 code, or a disk's own row to "
                             "see its boot code.");
        return {};
    }
    if (item->data(0, RoleOffset).isValid()) {
        why = QStringLiteral("A search hit is a place on the disk, not a program. Select a file "
                             "or the disk's own row.");
        return {};
    }
    if (!item->parent()) {
        // The disk's own row: its boot code.
        what = QStringLiteral("Boot code of %1").arg(open->name);
        return view < 0 ? open->image.bootCode()
                        : open->image.bootView(static_cast<AdeAsmView>(view));
    }
    if (item->data(0, RoleIsDir).toBool() || !item->data(0, RoleBlock).isValid()) {
        why = QStringLiteral("A folder holds no code. Select a program inside it.");
        return {};
    }
    what = item->text(ColName);
    const quint32 partition = item->data(0, RolePartition).toUInt();
    const quint32 block = item->data(0, RoleBlock).toUInt();
    if (view < 0) return open->image.disassemble(partition, block, m_asmForceRaw);
    return open->image.codeView(partition, block, static_cast<AdeAsmView>(view));
}

void MainWindow::refreshAssembly() {
    if (!m_asmStale || !m_asm) return;
    m_asmStale = false;
    m_asmRaw->hide();
    const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
    // A view follows the code from where it starts, which a guess at where
    // code is cannot give it; the explanations belong to the listing.
    if (m_asmExplain) m_asmExplain->setEnabled(view < 0);
    const auto say = [this](const QString &why) {
        m_asmShown = {};
        m_asm->clear();
        m_asmStatus->setText(why);
        if (m_asmSave) m_asmSave->setEnabled(false);
        if (m_asmBar) {
            m_asmBar->clear();
            m_asmBar->hide();
        }
        if (m_asmRefs) m_asmRefs->hide();
    };
    QString what;
    QString why;
    if (view >= 0 && m_asmForceRaw) {
        say(QStringLiteral("Decoded as a guess, so there is no start to follow: only the "
                           "listing is shown."));
        return;
    }
    ade::Disassembly listing = assemblyFor(view, what, why);
    if (!why.isEmpty()) {
        say(why);
        return;
    }
    if (!listing.read) {
        say(QStringLiteral("%1 could not be read.").arg(what));
        return;
    }
    if (!listing.refusal.isEmpty()) {
        say(QStringLiteral("%1 is %2.").arg(what, listing.refusal.startsWith(QStringLiteral("not "))
                                                      ? listing.refusal
                                                      : QStringLiteral("not shown: ") +
                                                            listing.refusal));
        // Offered for a file's listing, never for a disk: the boot code's
        // place is fixed by the format, and a disk that has none has none.
        if (m_selected && m_selected->parent() && view < 0) m_asmRaw->show();
        return;
    }

    m_asmShown = std::move(listing);
    m_asmWhat = what;
    if (m_asmSave) m_asmSave->setEnabled(true);
    // What refers to each line, for the strip; and the way back starts anew.
    m_asmReferrers.clear();
    for (int i = 0; i < m_asmShown.lines.size(); ++i) {
        for (quint64 t : m_asmShown.lines[i].targets) m_asmReferrers[t].append(i);
    }
    m_asmHistory.clear();
    m_asmHistoryAt = -1;
    m_asmBack->setEnabled(false);
    m_asmForward->setEnabled(false);
    // The bar goes with the listing, which is what it finds places in; a
    // guess has no walks to draw.
    if (m_asmBar) {
        QVector<SegmentBar::Run> runs;
        const Open *open = imageFor(m_selected);
        if (view < 0 && !m_asmForceRaw && open) {
            runs = m_selected->parent()
                       ? open->image.segments(m_selected->data(0, RolePartition).toUInt(),
                                              m_selected->data(0, RoleBlock).toUInt())
                       : open->image.bootSegments();
        }
        m_asmBar->setRuns(runs);
        m_asmBar->setVisible(!runs.isEmpty());
    }
    renderAssembly();
    // A name another view asked for, now the listing is here to hold it.
    if (!m_asmPendingName.isEmpty() && view < 0) {
        if (const std::optional<quint64> at = m_asmShown.locate(m_asmPendingName)) {
            showAssemblyOffset(*at);
            m_asmPendingName.clear();
        }
    }
    showAssemblyReferrers();
}

bool MainWindow::annotateAssemblyLine(int line, const QString &text, bool comment) {
    if (line < 0 || line >= m_asmShown.lines.size() || !m_asmShown.lines[line].hasOffset)
        return false;
    QTreeWidgetItem *item = m_selected;
    const Open *open = imageFor(item);
    if (!open) return false;
    // The disk's own row is its boot code: block 0.
    const quint32 block = item->parent() ? item->data(0, RoleBlock).toUInt() : 0;
    const quint32 partition = item->parent() ? item->data(0, RolePartition).toUInt() : 0;
    const QString why = open->image.annotate(partition, block, m_asmShown.lines[line].offset,
                                             text, comment);
    if (!why.isEmpty()) {
        emit errorOccurred(why);
        return false;
    }
    // Shown again with the change, where the reader was.
    const int top = m_asm->verticalScrollBar()->value();
    m_asmStale = true;
    refreshAssembly();
    m_asm->verticalScrollBar()->setValue(top);
    return true;
}

bool MainWindow::drawAssemblyGraph(int line, bool calls) {
    const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
    if (view >= 0 || m_asmForceRaw || line < 0 || line >= m_asmShown.lines.size()) return false;
    // A flow graph is of the function a line of code is in: the line itself,
    // or the first below a label.
    int at = line;
    while (!calls && at < m_asmShown.lines.size() && !m_asmShown.lines[at].hasOffset) ++at;
    if (!calls && at >= m_asmShown.lines.size()) return false;
    QTreeWidgetItem *item = m_selected;
    const Open *open = imageFor(item);
    if (!open) return false;
    const quint32 block = item->parent() ? item->data(0, RoleBlock).toUInt() : 0;
    const quint32 partition = item->parent() ? item->data(0, RolePartition).toUInt() : 0;
    const ade::Graph graph = calls ? open->image.callGraph(partition, block)
                                   : open->image.flowGraph(partition, block,
                                                           m_asmShown.lines[at].offset);
    if (!graph.refusal.isEmpty() || graph.nodes.isEmpty()) {
        emit errorOccurred(graph.refusal.isEmpty() ? QStringLiteral("There is nothing to draw.")
                                                   : graph.refusal);
        return false;
    }
    // A block shows its instructions, from the listing: the instruction
    // column of every line between its first byte and its end.
    QVector<GraphView::Node> nodes;
    for (int i = 0; i < graph.nodes.size(); ++i) {
        const AdeGraphNode &n = graph.nodes[i];
        GraphView::Node node{n.has_offset, n.offset, n.kind, n.layer, n.order, graph.labels[i], {}};
        if (!calls && n.has_offset) {
            for (const ade::AsmLine &l : m_asmShown.lines) {
                if (!l.hasOffset || l.offset < n.offset) continue;
                if (l.offset >= n.end) break;
                node.lines << l.text.mid(36).trimmed();
                if (node.lines.size() >= 40) {
                    node.lines << QStringLiteral("...");
                    break;
                }
            }
        }
        nodes.append(node);
    }
    QVector<GraphView::Edge> edges;
    for (const AdeGraphEdge &e : graph.edges) edges.append({e.from, e.to, e.kind, e.back});

    // One window, reused: a graph is looked at beside the listing.
    if (!m_graph) {
        m_graph = new QDialog(this);
        m_graph->setObjectName(QStringLiteral("assemblyGraph"));
        m_graph->resize(900, 700);
        auto *layout = new QVBoxLayout(m_graph);
        auto *bar = new QHBoxLayout;
        auto *what = new QLabel(m_graph);
        what->setObjectName(QStringLiteral("assemblyGraphWhat"));
        auto *view = new GraphView(m_graph);
        view->setObjectName(QStringLiteral("assemblyGraphView"));
        auto *fit = new QPushButton(QStringLiteral("Fit"), m_graph);
        auto *in = new QPushButton(QStringLiteral("+"), m_graph);
        auto *out = new QPushButton(QStringLiteral("-"), m_graph);
        connect(fit, &QPushButton::clicked, view, &GraphView::fit);
        connect(in, &QPushButton::clicked, view, [view] { view->zoom(1.25); });
        connect(out, &QPushButton::clicked, view, [view] { view->zoom(0.8); });
        connect(view, &GraphView::activated, this, [this](quint64 offset) {
            showAssemblyOffset(offset);
            activateWindow();
        });
        bar->addWidget(what, 1);
        bar->addWidget(fit);
        bar->addWidget(out);
        bar->addWidget(in);
        layout->addLayout(bar);
        layout->addWidget(view, 1);
        auto *legend = new QLabel(
            QStringLiteral("Green T: branch taken · red F: not taken · up the left: a loop · "
                           "double-click a box to go to it · Ctrl+wheel zooms, drag pans"),
            m_graph);
        legend->setWordWrap(true);
        layout->addWidget(legend);
    }
    auto *drawn = m_graph->findChild<GraphView *>(QStringLiteral("assemblyGraphView"));
    auto *what = m_graph->findChild<QLabel *>(QStringLiteral("assemblyGraphWhat"));
    drawn->setGraph(nodes, edges);
    const QString title = calls ? QStringLiteral("Call graph of %1").arg(m_asmWhat)
                                : QStringLiteral("Control flow of %1 in %2")
                                      .arg(graph.labels.value(0), m_asmWhat);
    what->setText(QStringLiteral("%1 — %2 boxes, %3 edges")
                      .arg(title)
                      .arg(nodes.size())
                      .arg(edges.size()));
    m_graph->setWindowTitle(title);
    m_graph->show();
    m_graph->raise();
    drawn->fit();
    return true;
}

void MainWindow::showAssemblyOffset(quint64 offset) {
    // The line holding the offset: the last one starting at or before it.
    int found = -1;
    for (int i = 0; i < m_asmShown.lines.size(); ++i) {
        const ade::AsmLine &line = m_asmShown.lines[i];
        if (!line.hasOffset) continue;
        if (line.offset > offset) break;
        found = i;
    }
    if (found >= 0) goToAssemblyLine(found);
}

void MainWindow::goToAssemblyLine(int line) {
    if (line < 0 || line >= m_asm->document()->blockCount()) return;
    // Where the reader was goes on the way back; anything ahead is dropped,
    // as a browser drops it.
    const int here = m_asm->textCursor().blockNumber();
    if (here != line) {
        m_asmHistory.resize(m_asmHistoryAt + 1);
        if (m_asmHistory.isEmpty() || m_asmHistory.last() != here) m_asmHistory.append(here);
        m_asmHistory.append(line);
        m_asmHistoryAt = static_cast<int>(m_asmHistory.size()) - 1;
    }
    // A block per line, as renderAssembly writes them.
    QTextCursor cursor(m_asm->document()->findBlockByNumber(line));
    m_asm->setTextCursor(cursor);
    m_asm->centerCursor();
    m_asmBack->setEnabled(m_asmHistoryAt > 0);
    m_asmForward->setEnabled(m_asmHistoryAt + 1 < m_asmHistory.size());
}

void MainWindow::stepAssemblyHistory(int step) {
    const int to = m_asmHistoryAt + step;
    if (to < 0 || to >= m_asmHistory.size()) return;
    m_asmHistoryAt = to;
    QTextCursor cursor(m_asm->document()->findBlockByNumber(m_asmHistory[to]));
    m_asm->setTextCursor(cursor);
    m_asm->centerCursor();
    m_asmBack->setEnabled(m_asmHistoryAt > 0);
    m_asmForward->setEnabled(m_asmHistoryAt + 1 < m_asmHistory.size());
}

bool MainWindow::goToAssemblyName(const QString &name) {
    const QString wanted = name.trimmed();
    if (wanted.isEmpty()) return false;
    const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
    if (view >= 0) {
        // From another view: the listing first, then the name in it.
        m_asmPendingName = wanted;
        m_asmShow->setCurrentIndex(m_asmShow->findData(-1));
        const bool found = m_asmPendingName.isEmpty();
        m_asmPendingName.clear();
        return found;
    }
    const std::optional<quint64> at = m_asmShown.locate(wanted);
    if (!at) return false;
    showAssemblyOffset(*at);
    return true;
}

void MainWindow::followAssemblyLine(int line) {
    if (line < 0 || line >= m_asmShown.lines.size()) return;
    const QVector<quint64> &targets = m_asmShown.lines[line].targets;
    if (targets.isEmpty()) return;
    if (targets.size() == 1) {
        showAssemblyOffset(targets.first());
        return;
    }
    // Several: say which, by the line each lands on.
    QMenu menu(this);
    for (quint64 target : targets) {
        QString shown = QStringLiteral("$%1").arg(target, 0, 16);
        for (const ade::AsmLine &l : m_asmShown.lines) {
            if (l.hasOffset && l.offset == target) {
                shown = l.text.simplified().left(60);
                break;
            }
        }
        QAction *go = menu.addAction(shown);
        connect(go, &QAction::triggered, this, [this, target] { showAssemblyOffset(target); });
    }
    menu.exec(m_asm->viewport()->mapToGlobal(m_asm->cursorRect().bottomLeft()));
}

void MainWindow::showAssemblyReferrers() {
    if (!m_asmRefs) return;
    const int line = m_asm->textCursor().blockNumber();
    const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
    // A label line refers to the line below it.
    int at = line;
    while (at >= 0 && at < m_asmShown.lines.size() && !m_asmShown.lines[at].hasOffset &&
           m_asmShown.lines[at].kind == ADE_ASM_LABEL) {
        ++at;
    }
    if (view >= 0 || at < 0 || at >= m_asmShown.lines.size() || !m_asmShown.lines[at].hasOffset) {
        m_asmRefs->hide();
        return;
    }
    const QVector<int> from = m_asmReferrers.value(m_asmShown.lines[at].offset);
    if (from.isEmpty()) {
        m_asmRefs->hide();
        return;
    }
    QStringList links;
    for (int i : from.mid(0, 24)) {
        const QString text = m_asmShown.lines[i].text;
        // The line's address and its instruction: enough to tell them apart.
        const QString shown =
            (text.left(8) + QLatin1Char(' ') + text.mid(36).section(QLatin1Char(' '), 0, 0))
                .toHtmlEscaped();
        links << QStringLiteral("<a href='%1'>%2</a>").arg(i).arg(shown);
    }
    if (from.size() > 24) links << QStringLiteral("and %1 more").arg(from.size() - 24);
    m_asmRefs->setText(QStringLiteral("Referred to from: %1").arg(links.join(QStringLiteral(", "))));
    m_asmRefs->show();
}

void MainWindow::findInAssembly() {
    if (m_asmFind.isEmpty()) return;
    if (!m_asm->find(m_asmFind)) {
        // From the top again, once.
        m_asm->moveCursor(QTextCursor::Start);
        if (!m_asm->find(m_asmFind)) {
            statusBar()->showMessage(QStringLiteral("%1 is not in the listing").arg(m_asmFind));
        }
    }
}

bool MainWindow::eventFilter(QObject *watched, QEvent *event) {
    if (m_asm && (watched == m_asm->viewport() || watched == m_asm)) {
        // The name under the cursor, as names are written: letters, digits,
        // `_` and `.`.
        const auto token = [this](const QTextCursor &at) {
            const QString text = at.block().text();
            int from = at.positionInBlock();
            int to = from;
            const auto part = [](QChar c) {
                return c.isLetterOrNumber() || c == QLatin1Char('_') || c == QLatin1Char('.');
            };
            while (from > 0 && part(text[from - 1])) --from;
            while (to < text.size() && part(text[to])) ++to;
            return text.mid(from, to - from);
        };
        const auto follow = [&](const QTextCursor &at) {
            // A name under the cursor first; else where the line goes.
            const QString name = token(at);
            if (!name.isEmpty() && goToAssemblyName(name)) return;
            followAssemblyLine(at.blockNumber());
        };
        if (watched == m_asm->viewport() && event->type() == QEvent::MouseButtonDblClick) {
            const auto *click = static_cast<QMouseEvent *>(event);
            follow(m_asm->cursorForPosition(click->position().toPoint()));
            return true;
        }
        if (watched == m_asm && event->type() == QEvent::KeyPress) {
            const auto *key = static_cast<QKeyEvent *>(event);
            if (key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) {
                follow(m_asm->textCursor());
                return true;
            }
            if ((key->modifiers() & Qt::AltModifier) && key->key() == Qt::Key_Left) {
                stepAssemblyHistory(-1);
                return true;
            }
            if ((key->modifiers() & Qt::AltModifier) && key->key() == Qt::Key_Right) {
                stepAssemblyHistory(1);
                return true;
            }
            if (key->key() == Qt::Key_Backspace) {
                stepAssemblyHistory(-1);
                return true;
            }
            if (key->key() == Qt::Key_G && key->modifiers() == Qt::NoModifier) {
                drawAssemblyGraph(m_asm->textCursor().blockNumber(), false);
                return true;
            }
        }
    }
    return QMainWindow::eventFilter(watched, event);
}

void MainWindow::saveAssembly() {
    if (m_asmShown.lines.isEmpty()) return;
    const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
    QString what;
    QString why;
    QStringList lines;
    QString suffix = QStringLiteral(".txt");
    if (view == int(ADE_ASM_CALL_GRAPH)) {
        // Saved for Graphviz, which is what a call graph is drawn with.
        const ade::Disassembly dot = assemblyFor(int(ADE_ASM_CALL_GRAPH_DOT), what, why);
        for (const ade::AsmLine &line : dot.lines) lines << line.text;
        suffix = QStringLiteral(".dot");
    } else {
        lines = m_asm->toPlainText().split(QLatin1Char('\n'));
        if (view == int(ADE_ASM_SOURCE)) suffix = QStringLiteral(".s");
    }
    if (lines.isEmpty()) return;
    QString base = m_asmWhat;
    base.replace(QLatin1Char('/'), QLatin1Char('_'));
    const QString target = QFileDialog::getSaveFileName(
        this, QStringLiteral("Save %1").arg(m_asmShow->currentText().toLower()),
        QDir::homePath() + QLatin1Char('/') + base + suffix);
    if (target.isEmpty()) return;
    QFile out(target);
    if (!out.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        emit errorOccurred(out.errorString());
        return;
    }
    out.write(lines.join(QLatin1Char('\n')).toUtf8());
    out.write("\n");
    if (!out.flush()) emit errorOccurred(out.errorString());
}

void MainWindow::renderAssembly() {
    if (!m_asm || m_asmShown.lines.isEmpty()) return;
    // The column the explanations start at, matching the CLI's listing.
    constexpr int CommentColumn = 72;
    const bool explain = m_asmExplain && m_asmExplain->isChecked();
    QStringList lines;
    lines.reserve(m_asmShown.lines.size());
    int instructions = 0;
    for (const ade::AsmLine &line : m_asmShown.lines) {
        // Explanations follow the checkbox; a note about the bytes themselves
        // ("not a 68000 instruction") is shown either way.
        const bool show = !line.comment.isEmpty() && (explain || line.kind != ADE_ASM_CODE);
        if (show) {
            const int width = qMax(CommentColumn, static_cast<int>(line.text.size()) + 2);
            lines << line.text.leftJustified(width) + QStringLiteral("; ") + line.comment;
        } else {
            lines << line.text;
        }
        if (line.kind == ADE_ASM_CODE) ++instructions;
    }
    // Where the reader was, kept across a toggle of the column.
    const int top = m_asm->verticalScrollBar()->value();
    m_asm->setPlainText(lines.join(QLatin1Char('\n')));
    m_asm->verticalScrollBar()->setValue(top);
    const int view = m_asmShow ? m_asmShow->currentData().toInt() : -1;
    if (view >= 0) {
        m_asmStatus->setText(QStringLiteral("%1 — %2").arg(m_asmWhat,
                                                            m_asmShow->currentText().toLower()));
        return;
    }
    m_asmStatus->setText(QStringLiteral("%1 — %2 instruction%3%4")
                             .arg(m_asmWhat)
                             .arg(instructions)
                             .arg(instructions == 1 ? "" : "s")
                             .arg(m_asmForceRaw ? QStringLiteral(", decoded as a guess")
                                                : QString{}));
}

// ---------------------------------------------------------------------------
// Filling only the preview that is showing (IMP-009)
// ---------------------------------------------------------------------------

void MainWindow::fillVisiblePreview() {
    if (!m_views) return;
    QWidget *showing = m_views->currentWidget();
    if (m_pending.text && showing == m_text) {
        m_pending.text = false;
        QByteArray bytes = m_pending.bytes;
        if (m_pending.disk) {
            // Capped: the text pane is a `QPlainTextEdit`, which lays out its
            // whole document when it is set — 4 MB takes over a second. Every
            // image in the corpus is smaller; only a hard disk is cut.
            constexpr quint64 WholeDiskText = 4u * 1024 * 1024;
            const ade::Buffer buffer = m_pending.disk->image.readRange(
                0, qMin(m_pending.disk->image.size(), WholeDiskText));
            if (buffer) bytes = buffer.data();
        }
        // Latin-1, like every other name and string off an Amiga disk.
        m_text->setPlainText(QString::fromLatin1(bytes));
    }
    if (!m_pending.text) m_pending = {};
}
