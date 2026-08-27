// The main window (F-004): a directory tree, a hex view, and a preview.
#pragma once

#include "Image.h"

#include <QMainWindow>

class QLabel;
class QPlainTextEdit;
class QSplitter;
class QTabWidget;
class QTreeWidget;
class QTreeWidgetItem;

class MainWindow : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(QWidget *parent = nullptr);

    // Open an image, replacing whatever is loaded. Failure is reported through
    // `errorOccurred` rather than shown here — see that signal.
    void openImage(const QString &path);

signals:
    // Something went wrong that a person should see.
    //
    // The window does not raise its own dialogs. A modal box blocks until it
    // is clicked, which makes every failure path untestable and, worse, means
    // the window decides how errors are surfaced for every future front end.
    // `main.cpp` connects this to a QMessageBox; the tests connect it to a
    // list, and so can check that failures are reported at all.
    void errorOccurred(const QString &message);

protected:
    // Drag-and-drop: dropping an image opens it (F-004).
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dropEvent(QDropEvent *event) override;

private slots:
    void chooseImage();
    void entrySelected();
    void extractSelected();

private:
    void populate(QTreeWidgetItem *parent, quint32 block);
    void showSummary();
    void clearViews();

    ade::Image m_image;
    QString m_path;

    QTreeWidget *m_tree = nullptr;
    QPlainTextEdit *m_hex = nullptr;
    QPlainTextEdit *m_text = nullptr;
    QTabWidget *m_views = nullptr;
    QLabel *m_summary = nullptr;
    QAction *m_extract = nullptr;
};
