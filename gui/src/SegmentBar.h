#pragma once

// A program drawn to scale beside its listing (F-036): which stretches the
// walks reached as code, which no walk reached, which are strings, data and
// zeroed memory. A listing shows a program a screen at a time; the bar shows
// where in all of it that screen is, and where the code nothing calls lies.
// Clicking a stretch goes to it in the listing.

#include <ade.h>

#include <QVector>
#include <QWidget>

class SegmentBar : public QWidget {
    Q_OBJECT

public:
    /// One run, as the engine gives it.
    struct Run {
        quint32 hunk = 0;
        quint32 start = 0;
        quint32 length = 0;
        AdeSegmentClass kind = ADE_SEG_DATA;
        bool hasOffset = false;
        quint64 offset = 0;
    };

    explicit SegmentBar(QWidget *parent = nullptr);

    void setRuns(const QVector<Run> &runs);
    void clear();
    const QVector<Run> &runs() const { return m_runs; }

    /// The colour a kind of run is drawn in, from the widget's palette, so
    /// it reads in a dark theme as in a light one.
    static QColor colourFor(const QPalette &palette, AdeSegmentClass kind);
    /// What a kind of run is, in words.
    static QString nameOf(AdeSegmentClass kind);

signals:
    /// A run with bytes in the file was clicked: the file offset under the
    /// pointer, as the listing's lines carry it.
    void jumpTo(quint64 offset);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    bool event(QEvent *event) override;

private:
    /// The run under `y`, and how far into it, in bytes; -1 for none.
    int runAt(int y, quint64 *into) const;

    QVector<Run> m_runs;
    quint64 m_total = 0;
};
