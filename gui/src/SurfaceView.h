#pragma once

// What came off each track of the medium (F-029).
//
// Two rows of eighty, because that is the shape of a floppy: the same cylinder
// on side 0 and side 1 are different tracks and fail independently. Drawing it
// as one long run of 160 would hide exactly the pattern that matters —
// `Realm of the Trolls` has one whole side readable and the other entirely
// blank, which is a sentence in this layout and noise in the other.

#include <ade.h>

#include <QVector>
#include <QWidget>

class SurfaceView : public QWidget {
    Q_OBJECT

public:
    explicit SurfaceView(QWidget *parent = nullptr);

    void setTracks(const QVector<AdeTrack> &tracks);

    /// The colour a track is drawn in, from what came off it.
    ///
    /// Four states, and the fourth is the one people forget: a track the
    /// container never mentioned is not the same as a track that yielded
    /// nothing. One is missing information, the other is information.
    static QColor colourFor(const QPalette &palette, const AdeTrack &track);

    /// The legend, as rich text.
    static QString legend(const QPalette &palette);

protected:
    void paintEvent(QPaintEvent *event) override;
    bool event(QEvent *event) override;

private:
    /// The cell rectangle for a track, or an empty rect if it has none.
    QRect cellFor(int cylinder, int head) const;
    /// The track under a point, or -1.
    int trackAt(const QPoint &where) const;

    QVector<AdeTrack> m_tracks;
};
