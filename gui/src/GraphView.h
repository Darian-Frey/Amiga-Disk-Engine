#pragma once

// A graph drawn as the engine laid it out (F-036): a function's control flow
// as boxes of code joined by its branches, or the program's call graph. The
// engine decides layers and order; this only scales them to pixels, draws,
// and lets the picture be panned, zoomed and clicked through to the listing.

#include <ade.h>

#include <QGraphicsView>
#include <QVector>

class GraphView : public QGraphicsView {
    Q_OBJECT

public:
    /// One node, as the engine gives it, with the lines to show in it.
    struct Node {
        bool hasOffset = false;
        quint64 offset = 0;
        quint32 kind = ADE_NODE_BLOCK;
        quint32 layer = 0;
        quint32 order = 0;
        QString label;
        QStringList lines;
    };
    /// One edge.
    struct Edge {
        quint32 from = 0;
        quint32 to = 0;
        quint32 kind = ADE_EDGE_NEXT;
        bool back = false;
    };

    explicit GraphView(QWidget *parent = nullptr);

    void setGraph(const QVector<Node> &nodes, const QVector<Edge> &edges);
    /// Fit the whole graph in the view.
    void fit();
    /// Zoom by `factor`, about the centre.
    void zoom(double factor);
    /// How many nodes and edges are drawn.
    int nodeCount() const { return m_nodes.size(); }
    int edgeCount() const { return m_edges.size(); }

signals:
    /// A node with bytes was double-clicked: its offset.
    void activated(quint64 offset);

protected:
    void wheelEvent(QWheelEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;

private:
    QVector<Node> m_nodes;
    QVector<Edge> m_edges;
};
