#include "MainWindow.h"

#include "ImageTree.h"

#include <QAction>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QFontMetrics>
#include <QHeaderView>
#include <QLabel>
#include <QMenuBar>
#include <QMimeData>
#include <QPlainTextEdit>
#include <QSplitter>
#include <QStatusBar>
#include <QTabWidget>
#include <QLineEdit>
#include <QTreeWidget>
#include <QUrl>
#include <QVBoxLayout>

namespace {

// Columns in the tree.
enum Column { ColName = 0, ColSize, ColDate, ColProtection };

// Which block an item stands for, and whether it is a directory.
constexpr int RoleBlock = Qt::UserRole + 1;
constexpr int RoleIsDir = Qt::UserRole + 2;
constexpr int RolePopulated = Qt::UserRole + 3;
// Which open image an item belongs to. Every item carries it, so a click in
// the tree or in the search results knows which disk it means.
constexpr int RoleImage = Qt::UserRole + 4;
// Which partition of that image, or ADE_WHOLE_IMAGE for one that holds its own
// volume. Carried by every item for the same reason: a block number means
// nothing without the volume it belongs to, and on a hard disk the same number
// is a different block in every partition.
constexpr int RolePartition = Qt::UserRole + 5;

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

QString hexDump(const QByteArray &data) {
    QString out;
    out.reserve(data.size() * 4);
    for (int offset = 0; offset < data.size(); offset += 16) {
        out += QStringLiteral("%1  ").arg(offset, 8, 16, QChar('0'));
        QString ascii;
        for (int i = 0; i < 16; ++i) {
            if (offset + i < data.size()) {
                const unsigned char c = static_cast<unsigned char>(data[offset + i]);
                out += QStringLiteral("%1 ").arg(c, 2, 16, QChar('0'));
                ascii += (c >= 0x20 && c < 0x7F) ? QChar(c) : QChar('.');
            } else {
                out += QStringLiteral("   ");
            }
            if (i == 7) out += QChar(' ');
        }
        out += QStringLiteral(" |") + ascii + QStringLiteral("|\n");
    }
    return out;
}

}  // namespace

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle(QStringLiteral("Amiga Disk Engine"));
    resize(1100, 700);
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

    m_hex = new QPlainTextEdit(this);
    m_hex->setReadOnly(true);
    m_hex->setLineWrapMode(QPlainTextEdit::NoWrap);
    m_hex->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));

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

    m_views = new QTabWidget(this);
    m_views->addTab(m_hex, QStringLiteral("Hex"));
    m_views->addTab(m_text, QStringLiteral("Text"));
    m_views->addTab(m_results, QStringLiteral("Search"));

    // The search box sits above the tree: it searches every open image, not
    // the selected one, so it belongs to the window rather than to a disk.
    m_query = new QLineEdit(this);
    m_query->setPlaceholderText(QStringLiteral("Search all open images by name..."));
    m_query->setClearButtonEnabled(true);
    connect(m_query, &QLineEdit::returnPressed, this, &MainWindow::search);

    auto *left = new QWidget(this);
    auto *leftLayout = new QVBoxLayout(left);
    leftLayout->setContentsMargins(0, 0, 0, 0);
    leftLayout->addWidget(m_query);
    leftLayout->addWidget(m_tree);

    auto *splitter = new QSplitter(Qt::Horizontal, this);
    splitter->addWidget(left);
    splitter->addWidget(m_views);
    splitter->setStretchFactor(0, 1);
    splitter->setStretchFactor(1, 1);
    setCentralWidget(splitter);

    // The tree extracts through the window, which owns the images; the tree
    // itself knows nothing about them.
    const auto extractor = [this](QTreeWidgetItem *item) { return contentsOf(item); };
    m_tree->setExtractor(extractor);
    m_results->setExtractor(extractor);

    m_summary = new QLabel(this);
    statusBar()->addPermanentWidget(m_summary);

    auto *file = menuBar()->addMenu(QStringLiteral("&File"));
    auto *open = file->addAction(QStringLiteral("&Open image..."));
    open->setShortcut(QKeySequence::Open);
    connect(open, &QAction::triggered, this, &MainWindow::chooseImage);

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

void MainWindow::search() {
    m_results->clear();
    const QString query = m_query->text().trimmed();
    if (query.isEmpty()) {
        statusBar()->showMessage(QStringLiteral("Type a name to search for"));
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
    // An image row carries no block. Selecting one says what the image is,
    // which is the part its own row may have had to elide.
    if (item && !item->data(0, RoleBlock).isValid()) {
        clearViews();
        if (const Open *open = imageFor(item)) statusBar()->showMessage(describe(*open));
        return;
    }
    if (!item || item->data(0, RoleIsDir).toBool()) {
        clearViews();
        return;
    }
    m_extract->setEnabled(true);

    const QByteArray data = contentsOf(item);
    if (data.isEmpty()) {
        // Either unreadable or genuinely empty. Both are worth saying, and
        // neither is worth a dialog.
        m_hex->setPlainText(QStringLiteral("(no readable contents)"));
        m_text->clear();
        return;
    }
    const QByteArray head = data.left(PreviewBytes);

    m_hex->setPlainText(hexDump(head));
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
    if (m_extract) m_extract->setEnabled(false);
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
