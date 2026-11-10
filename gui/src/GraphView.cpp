#include "GraphView.h"

#include <QFontDatabase>
#include <QFontMetrics>
#include <QGraphicsPathItem>
#include <QGraphicsRectItem>
#include <QGraphicsScene>
#include <QGraphicsSimpleTextItem>
#include <QMouseEvent>
#include <QPainterPath>
#include <QWheelEvent>
#include <QtMath>

namespace {

constexpr int Pad = 6;        // inside a box
constexpr int ColumnGap = 40;  // between boxes in a layer
constexpr int LayerGap = 50;   // between layers
constexpr int Left = 60;       // room for loops routed up the left

// A colour mixed from `a` towards `b` by `t`.
QColor mix(const QColor &a, const QColor &b, qreal t) {
    return QColor::fromRgbF(a.redF() * (1 - t) + b.redF() * t, a.greenF() * (1 - t) + b.greenF() * t,
                            a.blueF() * (1 - t) + b.blueF() * t);
}

}  // namespace

GraphView::GraphView(QWidget *parent) : QGraphicsView(parent) {
    setScene(new QGraphicsScene(this));
    setDragMode(QGraphicsView::ScrollHandDrag);
    setRenderHint(QPainter::Antialiasing);
    setTransformationAnchor(QGraphicsView::AnchorUnderMouse);
    // From the top, where a graph starts, when it is shorter than the view.
    setAlignment(Qt::AlignHCenter | Qt::AlignTop);
}

void GraphView::setGraph(const QVector<Node> &nodes, const QVector<Edge> &edges) {
    m_nodes = nodes;
    m_edges = edges;
    QGraphicsScene *s = scene();
    s->clear();
    const QPalette p = palette();
    const QColor text = p.color(QPalette::Text);
    const QColor base = p.color(QPalette::Base);
    const QFont mono = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    const QFontMetrics metrics(mono);

    // Each box's size, from its text.
    QVector<QSizeF> size(m_nodes.size());
    for (int i = 0; i < m_nodes.size(); ++i) {
        const Node &n = m_nodes[i];
        int width = metrics.horizontalAdvance(n.label);
        for (const QString &line : n.lines) width = qMax(width, metrics.horizontalAdvance(line));
        const int rows = 1 + static_cast<int>(n.lines.size());
        size[i] = QSizeF(width + 2 * Pad, rows * metrics.height() + 2 * Pad);
    }
    // Layers top to bottom, each as tall as its tallest box; within a layer,
    // boxes left to right in the engine's order, the row centred.
    QMap<quint32, QVector<int>> layers;
    for (int i = 0; i < m_nodes.size(); ++i) layers[m_nodes[i].layer].append(i);
    double widest = 0;
    QMap<quint32, double> rowWidth;
    for (auto it = layers.begin(); it != layers.end(); ++it) {
        std::sort(it->begin(), it->end(),
                  [this](int a, int b) { return m_nodes[a].order < m_nodes[b].order; });
        double w = 0;
        for (int i : *it) w += size[i].width() + ColumnGap;
        rowWidth[it.key()] = w;
        widest = qMax(widest, w);
    }
    QVector<QRectF> box(m_nodes.size());
    double y = 0;
    for (auto it = layers.begin(); it != layers.end(); ++it) {
        double x = Left + (widest - rowWidth[it.key()]) / 2;
        double tallest = 0;
        for (int i : *it) {
            box[i] = QRectF(QPointF(x, y), size[i]);
            x += size[i].width() + ColumnGap;
            tallest = qMax(tallest, size[i].height());
        }
        y += tallest + LayerGap;
    }

    // Edges first, under the boxes.
    const QColor neutral = mix(text, base, 0.35);
    for (const Edge &e : m_edges) {
        if (e.from >= static_cast<quint32>(box.size()) || e.to >= static_cast<quint32>(box.size()))
            continue;
        const QRectF a = box[static_cast<int>(e.from)];
        const QRectF b = box[static_cast<int>(e.to)];
        QColor colour = neutral;
        if (e.kind == ADE_EDGE_TAKEN) colour = mix(QColor(40, 170, 80), text, 0.25);
        if (e.kind == ADE_EDGE_NOT_TAKEN) colour = mix(QColor(220, 70, 60), text, 0.25);
        QPainterPath path;
        QPointF end;
        if (e.back) {
            // Up the left: out of the source's side, round, into the target's.
            const QPointF from(a.left(), a.center().y());
            end = QPointF(b.left(), b.top() + Pad + metrics.height() / 2.0);
            const double out = qMin(a.left(), b.left()) - 25;
            path.moveTo(from);
            path.cubicTo(QPointF(out, from.y()), QPointF(out, end.y()), end);
        } else {
            const QPointF from(a.center().x(), a.bottom());
            end = QPointF(b.center().x(), b.top());
            // A long edge would pass behind the boxes of the layers it
            // skips: it goes round the side of any in its way instead.
            double around = -1;
            const double lo = qMin(from.x(), end.x()) - 10;
            const double hi = qMax(from.x(), end.x()) + 10;
            for (int k = 0; k < box.size(); ++k) {
                const QRectF &c = box[k];
                if (k == static_cast<int>(e.from) || k == static_cast<int>(e.to)) continue;
                if (c.top() > from.y() && c.bottom() < end.y() && c.right() > lo && c.left() < hi) {
                    around = qMax(around, c.right() + 15);
                }
            }
            path.moveTo(from);
            if (around < 0) {
                path.cubicTo(QPointF(from.x(), from.y() + LayerGap / 2.0),
                             QPointF(end.x(), end.y() - LayerGap / 2.0), end);
            } else {
                const double top = from.y() + LayerGap / 2.0;
                const double bottom = end.y() - LayerGap / 2.0;
                path.cubicTo(QPointF(from.x(), top), QPointF(around, from.y()),
                             QPointF(around, top + LayerGap / 2.0));
                path.lineTo(QPointF(around, bottom - LayerGap / 2.0));
                path.cubicTo(QPointF(around, end.y()), QPointF(end.x(), bottom), end);
            }
        }
        auto *line = s->addPath(path, QPen(colour, e.kind == ADE_EDGE_CALL ? 1.0 : 1.5));
        line->setZValue(0);
        // An arrowhead at the end, along the curve's last direction.
        const double angle = path.angleAtPercent(1.0);
        const QPointF tip = end;
        const QPointF left = tip + QPointF(qCos(qDegreesToRadians(angle + 150)) * 8,
                                           -qSin(qDegreesToRadians(angle + 150)) * 8);
        const QPointF right = tip + QPointF(qCos(qDegreesToRadians(angle - 150)) * 8,
                                            -qSin(qDegreesToRadians(angle - 150)) * 8);
        QPolygonF head;
        head << tip << left << right;
        s->addPolygon(head, QPen(colour), QBrush(colour));
        // T and F where a conditional branch leaves its block.
        if (e.kind == ADE_EDGE_TAKEN || e.kind == ADE_EDGE_NOT_TAKEN) {
            auto *mark = s->addSimpleText(e.kind == ADE_EDGE_TAKEN ? QStringLiteral("T")
                                                                  : QStringLiteral("F"));
            mark->setBrush(colour);
            mark->setPos(path.pointAtPercent(0.12) + QPointF(4, -4));
        }
    }

    // The boxes: a header with the label, then the lines.
    for (int i = 0; i < m_nodes.size(); ++i) {
        const Node &n = m_nodes[i];
        QColor head = mix(text, base, 0.82);
        if (n.kind == ADE_NODE_ENTRY) head = mix(QColor(70, 130, 220), base, 0.45);
        if (n.kind == ADE_NODE_EXIT) head = mix(QColor(40, 170, 80), base, 0.5);
        if (n.kind == ADE_NODE_LIBRARY) head = mix(QColor(170, 80, 200), base, 0.5);
        auto *rect = s->addRect(box[i], QPen(mix(text, base, 0.45)), QBrush(base));
        rect->setZValue(1);
        rect->setData(0, i);
        auto *header = s->addRect(QRectF(box[i].topLeft(),
                                         QSizeF(box[i].width(), metrics.height() + Pad)),
                                  Qt::NoPen, QBrush(head));
        header->setZValue(2);
        header->setData(0, i);
        auto *label = s->addSimpleText(n.label, mono);
        label->setBrush(text);
        label->setPos(box[i].topLeft() + QPointF(Pad, Pad / 2.0));
        label->setZValue(3);
        label->setData(0, i);
        if (!n.lines.isEmpty()) {
            auto *body = s->addSimpleText(n.lines.join(QLatin1Char('\n')), mono);
            body->setBrush(text);
            body->setPos(box[i].topLeft() + QPointF(Pad, metrics.height() + Pad * 1.5));
            body->setZValue(3);
            body->setData(0, i);
        }
    }
    s->setSceneRect(s->itemsBoundingRect().adjusted(-20, -20, 20, 20));
}

void GraphView::fit() {
    fitInView(scene()->sceneRect(), Qt::KeepAspectRatio);
    // Never larger than life: a small graph is shown at its own size. Nor
    // too small to read: a wide one is shown at a readable size from the
    // top, where it starts, and scrolled for the rest.
    constexpr double Readable = 0.6;
    if (transform().m11() > 1.0) {
        resetTransform();
    } else if (transform().m11() < Readable) {
        resetTransform();
        scale(Readable, Readable);
        const QRectF all = scene()->sceneRect();
        centerOn(all.center().x(), all.top() + viewport()->height() / (2 * Readable));
    }
}

void GraphView::zoom(double factor) {
    const double now = transform().m11();
    if ((now * factor) < 0.05 || (now * factor) > 4) return;
    scale(factor, factor);
}

void GraphView::wheelEvent(QWheelEvent *event) {
    if (event->modifiers() & Qt::ControlModifier) {
        zoom(event->angleDelta().y() > 0 ? 1.15 : 1 / 1.15);
        event->accept();
        return;
    }
    QGraphicsView::wheelEvent(event);
}

void GraphView::mouseDoubleClickEvent(QMouseEvent *event) {
    for (QGraphicsItem *item : items(event->position().toPoint())) {
        bool ok = false;
        const int i = item->data(0).toInt(&ok);
        if (ok && i >= 0 && i < m_nodes.size() && m_nodes[i].hasOffset) {
            emit activated(m_nodes[i].offset);
            return;
        }
    }
    QGraphicsView::mouseDoubleClickEvent(event);
}
