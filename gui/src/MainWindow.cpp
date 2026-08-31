#include "MainWindow.h"

#include "HexView.h"
#include "ImageTree.h"

#include <QAction>
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
#include <QLineEdit>
#include <QTimer>
#include <QTreeWidget>
#include <QUrl>
#include <QVBoxLayout>

namespace {
using namespace tree;


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
int hexPaneWidth(const QPlainTextEdit &pane) {
    const QFontMetrics metrics(pane.font());
    const int text = metrics.horizontalAdvance(QString(hexview::LineLength, QChar('0')));
    // The document's own left and right margins, the frame, and room for the
    // vertical scrollbar a full disk will always need.
    const int margins = static_cast<int>(pane.document()->documentMargin()) * 2;
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

    m_hex = new HexPane(this);
    m_hex->setObjectName(QStringLiteral("hex"));
    m_hex->setReadOnly(true);
    m_hex->setLineWrapMode(QPlainTextEdit::NoWrap);
    m_hex->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    // Scrolling the whole disk says which file is on screen. Not by selecting
    // its row: selection is what *chose* the whole-disk view, so following the
    // scroll with it would replace the view being scrolled — the feature would
    // undo itself on the first wheel click. The row is marked instead, which
    // is a different thing said a different way.
    connect(m_hex, &HexPane::scrolled, this, [this] { markWhatIsOnScreen(); });

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

    // Null bytes dimmed and disk regions tinted. Owned by the document, which
    // outlives every dump put into it, and kept so the regions can be set as
    // the selection changes.
    m_paint = new HexHighlighter(m_hex->document(), m_hex);

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
    m_views->addTab(hexTab, QStringLiteral("Hex"));
    m_views->addTab(m_text, QStringLiteral("Text"));
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
        emit errorOccurred(QStringLiteral("%1 is not a disk image.")
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

void MainWindow::closeAll() {
    // The trees first: their items hold indices into `m_images`, and an item
    // outliving the image it points at is the one way this can dangle.
    m_tree->clear();
    m_results->clear();
    m_selected = nullptr;
    m_images.clear();
    clearViews();
    setWindowTitle(QStringLiteral("Amiga Disk Engine"));
    m_summary->clear();
    statusBar()->showMessage(QStringLiteral("Open a disk image, or drop one here"));
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
    const QString named = open.image.identified();
    root->setToolTip(ColName,
                     named.isEmpty()
                         ? QStringLiteral("%1\n%2").arg(open.path, describe(open))
                         : QStringLiteral("%1\n%2\n%3").arg(open.path, named, describe(open)));

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

    if (!open.image.hasVolume()) return;
    root->setData(0, RolePartition, ADE_WHOLE_IMAGE);
    populate(root, open, ADE_WHOLE_IMAGE, open.image.rootBlock());
    root->setExpanded(true);
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
        parts << (open.image.hasVolume() ? QStringLiteral("\"%1\"").arg(open.image.volumeName())
                                         : open.image.volumeAbsent());
    }
    parts << QStringLiteral("%1 bytes").arg(open.image.size());
    const size_t findings = open.image.findingCount();
    if (findings > 0) {
        parts << QStringLiteral("%1 finding%2").arg(findings).arg(findings == 1 ? "" : "s");
    }
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
    setWindowTitle(QStringLiteral("%1 — Amiga Disk Engine").arg(only.name));
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
        auto *item = parent ? new QTreeWidgetItem(parent) : new QTreeWidgetItem(m_tree);
        item->setText(ColName, ade::latin1(entry.name));
        item->setText(ColSize, isDir ? QString{} : QString::number(entry.size));
        item->setText(ColDate, formatDate(entry.days, entry.mins, entry.ticks));
        item->setText(ColProtection, formatProtection(entry.protection));
        item->setTextAlignment(ColSize, Qt::AlignRight | Qt::AlignVCenter);
        item->setData(0, RoleBlock, entry.block);
        item->setData(0, RoleIsDir, isDir);
        item->setData(0, RoleImage, static_cast<qulonglong>(open.index));
        item->setData(0, RolePartition, partition);

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
    megabytes->setRange(1, 49);  // past 49 a bitmap extension chain is needed
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
    openImage(path);
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
        const QColor colour = HexHighlighter::regionColour(m_hex->palette(), region);
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
    m_views->setCurrentWidget(m_hexTab);
    markWhatIsOnScreen();
}

void MainWindow::showWholeDisk(QTreeWidgetItem *item) {
    const Open *open = imageFor(item);
    if (!open) {
        clearViews();
        return;
    }
    const quint64 size = open->image.size();
    // Capped, and the cap is measured rather than chosen. A dump is five
    // characters per byte, and Qt lays out the document when it is set: 4 MB
    // takes about 1.4 seconds, 2 MB about 0.7. Four is the number because
    // every image in the 4,652-image corpus is smaller than that — the largest
    // is 2.1 MB — so every floppy, extended ADF and flux capture is shown
    // whole, and only a hard disk is cut. Which is the right place to cut: a
    // hex dump of a hundred megabytes was never the way to read one.
    constexpr quint64 WholeDiskBytes = 4u * 1024 * 1024;
    const quint64 shown = qMin(size, WholeDiskBytes);

    const ade::Buffer buffer = open->image.readRange(0, shown);
    if (!buffer) {
        clearViews();
        return;
    }
    const QByteArray bytes = buffer.data();

    // The map goes in **before** the text, over a cleared pane. A
    // QSyntaxHighlighter re-highlights its whole document when its inputs
    // change, and a whole disk is 56,000 lines: setting the map afterwards
    // made the GUI test suite take eleven seconds instead of ninety
    // milliseconds, and left the pane briefly painted with the previous
    // disk's regions. Cleared, then mapped, then filled — nothing is
    // highlighted twice, and nothing is ever highlighted wrongly.
    const QVector<HexRegion> regions = open->image.regions();
    m_hex->clear();
    if (m_paint) m_paint->setRegions(regions);
    m_diskRegions = regions;
    m_diskImage = item->data(0, RoleImage).toULongLong();
    m_diskPartition = item->data(0, RolePartition).toUInt();
    m_topLine = -1;
    if (m_follow) m_follow->start();
    m_hex->setPlainText(hexview::dump(bytes));
    m_text->setPlainText(QString::fromLatin1(bytes));
    showLegend(regions);
    m_extract->setEnabled(false);

    QString message = describe(*open);
    if (shown < size) {
        message += QStringLiteral("  —  hex shows the first %1 of %2 bytes")
                       .arg(shown)
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
    // Extracting everything needs a disk, not a file: any row of any image
    // will do, because the whole disk is what it takes.
    if (m_extractAll) m_extractAll->setEnabled(imageFor(item) != nullptr);
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
    if (m_paint) m_paint->setRegions({});
    m_diskRegions.clear();
    if (m_follow) m_follow->stop();
    markRow(0);
    showLegend({});

    const QByteArray data = contentsOf(item);
    if (data.isEmpty()) {
        // Either unreadable or genuinely empty. Both are worth saying, and
        // neither is worth a dialog.
        m_hex->setPlainText(QStringLiteral("(no readable contents)"));
        m_text->clear();
        return;
    }
    const QByteArray head = data.left(PreviewBytes);

    m_hex->setPlainText(hexview::dump(head));
    // Latin-1, like every other name and string off an Amiga disk.
    m_text->setPlainText(QString::fromLatin1(head));
    if (data.size() > head.size()) {
        statusBar()->showMessage(
            QStringLiteral("Showing the first %1 of %2 bytes").arg(head.size()).arg(data.size()));
    } else {
        statusBar()->showMessage(QStringLiteral("%1 bytes").arg(data.size()));
    }
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
    m_hex->clear();
    m_text->clear();
    // The map and its legend belong to whatever was being shown. Left behind,
    // they would explain colours that are no longer on screen — and worse, the
    // map would tint the next thing put in the pane by where *this* one's
    // offsets fell.
    if (m_paint) m_paint->setRegions({});
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
