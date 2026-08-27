#include "MainWindow.h"

#include <QAction>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QFileDialog>
#include <QFileInfo>
#include <QHeaderView>
#include <QLabel>
#include <QMenuBar>
#include <QMimeData>
#include <QPlainTextEdit>
#include <QSplitter>
#include <QStatusBar>
#include <QTabWidget>
#include <QTreeWidget>
#include <QVBoxLayout>

namespace {

// Columns in the tree.
enum Column { ColName = 0, ColSize, ColDate, ColProtection };

// Which block an item stands for, and whether it is a directory.
constexpr int RoleBlock = Qt::UserRole + 1;
constexpr int RoleIsDir = Qt::UserRole + 2;
constexpr int RolePopulated = Qt::UserRole + 3;

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

    m_tree = new QTreeWidget(this);
    m_tree->setColumnCount(4);
    m_tree->setHeaderLabels({QStringLiteral("Name"), QStringLiteral("Size"),
                             QStringLiteral("Modified"), QStringLiteral("Protection")});
    m_tree->header()->setStretchLastSection(false);
    m_tree->header()->setSectionResizeMode(ColName, QHeaderView::Stretch);
    // The other three hold fixed-width content — a size, a timestamp, and
    // eight protection flags — so size them to it. Left to stretch they
    // truncate the date to "1990-09-20 17:...", which is the one part of it
    // nobody can infer.
    for (int column : {ColSize, ColDate, ColProtection}) {
        m_tree->header()->setSectionResizeMode(column, QHeaderView::ResizeToContents);
    }
    m_tree->setUniformRowHeights(true);

    m_hex = new QPlainTextEdit(this);
    m_hex->setReadOnly(true);
    m_hex->setLineWrapMode(QPlainTextEdit::NoWrap);
    m_hex->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));

    m_text = new QPlainTextEdit(this);
    m_text->setReadOnly(true);
    m_text->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));

    m_views = new QTabWidget(this);
    m_views->addTab(m_hex, QStringLiteral("Hex"));
    m_views->addTab(m_text, QStringLiteral("Text"));

    auto *splitter = new QSplitter(Qt::Horizontal, this);
    splitter->addWidget(m_tree);
    splitter->addWidget(m_views);
    splitter->setStretchFactor(0, 1);
    splitter->setStretchFactor(1, 1);
    setCentralWidget(splitter);

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
    auto *quit = file->addAction(QStringLiteral("&Quit"));
    quit->setShortcut(QKeySequence::Quit);
    connect(quit, &QAction::triggered, this, &QWidget::close);

    connect(m_tree, &QTreeWidget::itemSelectionChanged, this, &MainWindow::entrySelected);
    connect(m_tree, &QTreeWidget::itemExpanded, this, [this](QTreeWidgetItem *item) {
        if (item->data(0, RolePopulated).toBool()) return;
        item->setData(0, RolePopulated, true);
        populate(item, item->data(0, RoleBlock).toUInt());
    });

    statusBar()->showMessage(QStringLiteral("Open a disk image, or drop one here"));
}

void MainWindow::chooseImage() {
    const QString path = QFileDialog::getOpenFileName(
        this, QStringLiteral("Open a disk image"), {},
        QStringLiteral("Disk images (*.adf *.adz *.hdf *.hdz *.dms);;All files (*)"));
    if (!path.isEmpty()) openImage(path);
}

void MainWindow::openImage(const QString &path) {
    ade::Image image = ade::Image::open(path);
    if (!image) {
        emit errorOccurred(QStringLiteral("%1: ADE could not read this file (error %2).")
                               .arg(QFileInfo(path).fileName())
                               .arg(static_cast<int>(image.error())));
        return;
    }

    m_image = std::move(image);
    m_path = path;
    clearViews();
    m_tree->clear();

    setWindowTitle(QStringLiteral("%1 — Amiga Disk Engine").arg(QFileInfo(path).fileName()));
    showSummary();

    if (!m_image.hasVolume()) {
        // Not an error: a quarter of real images are not AmigaDOS disks, and
        // the container is still worth showing.
        statusBar()->showMessage(m_image.volumeAbsent());
        return;
    }
    populate(nullptr, m_image.rootBlock());
    statusBar()->showMessage(QStringLiteral("Opened %1").arg(QFileInfo(path).fileName()));
}

void MainWindow::showSummary() {
    QStringList parts;
    parts << m_image.container();
    if (m_image.hasVolume()) {
        parts << QStringLiteral("\"%1\"").arg(m_image.volumeName());
    }
    parts << QStringLiteral("%1 bytes").arg(m_image.size());
    const size_t findings = m_image.findingCount();
    if (findings > 0) {
        parts << QStringLiteral("%1 finding%2").arg(findings).arg(findings == 1 ? "" : "s");
    }
    m_summary->setText(parts.join(QStringLiteral("   ")));
}

void MainWindow::populate(QTreeWidgetItem *parent, quint32 block) {
    const ade::Listing listing = m_image.list(block);
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

        if (isDir) {
            // A placeholder child makes the expander appear; the real children
            // are read when it is opened. Walking a whole disk up front is
            // wasted work on an image the user only wants to glance at.
            item->setData(0, RolePopulated, false);
            item->setChildIndicatorPolicy(QTreeWidgetItem::ShowIndicator);
        }
    }
    m_tree->sortItems(ColName, Qt::AscendingOrder);
}

void MainWindow::entrySelected() {
    const auto selected = m_tree->selectedItems();
    if (selected.isEmpty()) {
        clearViews();
        return;
    }
    QTreeWidgetItem *item = selected.first();
    const bool isDir = item->data(0, RoleIsDir).toBool();
    m_extract->setEnabled(!isDir);
    if (isDir) {
        clearViews();
        return;
    }

    const ade::Buffer buffer = m_image.read(item->data(0, RoleBlock).toUInt());
    if (!buffer) {
        m_hex->setPlainText(QStringLiteral("(this file could not be read)"));
        m_text->clear();
        return;
    }
    const QByteArray data = buffer.data();
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
    const auto selected = m_tree->selectedItems();
    if (selected.isEmpty()) return;
    QTreeWidgetItem *item = selected.first();
    if (item->data(0, RoleIsDir).toBool()) return;

    const ade::Buffer buffer = m_image.read(item->data(0, RoleBlock).toUInt());
    if (!buffer) {
        emit errorOccurred(QStringLiteral("This file could not be read."));
        return;
    }
    const QString target = QFileDialog::getSaveFileName(
        this, QStringLiteral("Extract to"), item->text(ColName));
    if (target.isEmpty()) return;

    QFile out(target);
    if (!out.open(QIODevice::WriteOnly)) {
        emit errorOccurred(out.errorString());
        return;
    }
    const QByteArray data = buffer.data();
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
    const QList<QUrl> urls = event->mimeData()->urls();
    if (urls.isEmpty()) return;
    const QString path = urls.first().toLocalFile();
    if (!path.isEmpty()) openImage(path);
}
