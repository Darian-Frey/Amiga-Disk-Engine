// RAII wrappers over the C ABI (D-001).
//
// The ABI hands out raw pointers with explicit free functions, which is right
// for a C interface and wrong to use directly from C++. Everything below owns
// exactly one handle and releases it once, so no path through the UI can leak
// or double-free — including the ones that throw.

#pragma once

#include <ade.h>

#include "HexView.h"

#include <QByteArray>
#include <QString>

#include <utility>

namespace ade {

// Amiga names are Latin-1, and the ABI hands them over as bytes precisely so
// the caller decides. This is that decision, in one place: every name in the
// UI goes through here.
inline QString latin1(AdeBytes bytes) {
    if (!bytes.data || bytes.len == 0) return {};
    return QString::fromLatin1(reinterpret_cast<const char *>(bytes.data),
                               static_cast<int>(bytes.len));
}

inline QByteArray bytes(AdeBytes b) {
    if (!b.data || b.len == 0) return {};
    return QByteArray(reinterpret_cast<const char *>(b.data), static_cast<int>(b.len));
}

// A directory listing. Move-only: two owners would free it twice.
class Listing {
public:
    Listing() = default;
    explicit Listing(AdeListing *raw) : m_raw(raw) {}
    ~Listing() { ade_listing_free(m_raw); }

    Listing(const Listing &) = delete;
    Listing &operator=(const Listing &) = delete;
    Listing(Listing &&other) noexcept : m_raw(std::exchange(other.m_raw, nullptr)) {}
    Listing &operator=(Listing &&other) noexcept {
        if (this != &other) {
            ade_listing_free(m_raw);
            m_raw = std::exchange(other.m_raw, nullptr);
        }
        return *this;
    }

    explicit operator bool() const { return m_raw != nullptr; }
    size_t count() const { return ade_listing_count(m_raw); }

    bool entry(size_t index, AdeEntry *out) const {
        return ade_listing_entry(m_raw, index, out) == ADE_OK;
    }

private:
    AdeListing *m_raw = nullptr;
};

// A device's partition table. Move-only, like every other handle.
/// A content search over one image (F-021).
///
/// Unlike every other handle here, one of these exists even when the search
/// could not run: `error()` is then non-empty and `count()` is zero. That is
/// the whole point — "the pattern was refused" and "the pattern is not on this
/// disk" are different answers, and a null handle would collapse them.
class Search {
public:
    Search() = default;
    explicit Search(AdeSearch *raw) : m_raw(raw) {}
    ~Search() { ade_find_free(m_raw); }

    Search(const Search &) = delete;
    Search &operator=(const Search &) = delete;
    Search(Search &&other) noexcept : m_raw(std::exchange(other.m_raw, nullptr)) {}
    Search &operator=(Search &&other) noexcept {
        if (this != &other) {
            ade_find_free(m_raw);
            m_raw = std::exchange(other.m_raw, nullptr);
        }
        return *this;
    }

    explicit operator bool() const { return m_raw != nullptr; }
    size_t count() const { return ade_find_count(m_raw); }
    bool wasHex() const { return ade_find_was_hex(m_raw); }
    /// Why the pattern was refused; empty when it was not.
    QString error() const { return latin1(ade_find_error(m_raw)); }

    bool at(size_t index, AdeMatch *out) const {
        return ade_find_match(m_raw, index, out) == ADE_OK;
    }

private:
    AdeSearch *m_raw = nullptr;
};

class Partitions {
public:
    Partitions() = default;
    explicit Partitions(AdePartitions *raw) : m_raw(raw) {}
    ~Partitions() { ade_partitions_free(m_raw); }

    Partitions(const Partitions &) = delete;
    Partitions &operator=(const Partitions &) = delete;
    Partitions(Partitions &&other) noexcept : m_raw(std::exchange(other.m_raw, nullptr)) {}
    Partitions &operator=(Partitions &&other) noexcept {
        if (this != &other) {
            ade_partitions_free(m_raw);
            m_raw = std::exchange(other.m_raw, nullptr);
        }
        return *this;
    }

    explicit operator bool() const { return m_raw != nullptr; }
    size_t count() const { return ade_partitions_count(m_raw); }

    bool at(size_t index, AdePartition *out) const {
        return ade_partitions_entry(m_raw, index, out) == ADE_OK;
    }

private:
    AdePartitions *m_raw = nullptr;
};

// A file's contents.
class Buffer {
public:
    Buffer() = default;
    explicit Buffer(AdeBuffer *raw) : m_raw(raw) {}
    ~Buffer() { ade_buffer_free(m_raw); }

    Buffer(const Buffer &) = delete;
    Buffer &operator=(const Buffer &) = delete;
    Buffer(Buffer &&other) noexcept : m_raw(std::exchange(other.m_raw, nullptr)) {}
    Buffer &operator=(Buffer &&other) noexcept {
        if (this != &other) {
            ade_buffer_free(m_raw);
            m_raw = std::exchange(other.m_raw, nullptr);
        }
        return *this;
    }

    explicit operator bool() const { return m_raw != nullptr; }
    QByteArray data() const { return bytes(ade_buffer_bytes(m_raw)); }

private:
    AdeBuffer *m_raw = nullptr;
};

// A dataset of datfiles, loaded once for the session.
//
// Loading 88,921 entries takes about 140 ms, which is why the window holds one
// rather than consulting the dataset per image: paid at startup, spent on
// every disk opened afterwards (F-013).
class Catalogue {
public:
    Catalogue() = default;
    explicit Catalogue(AdeCatalogue *raw) : m_raw(raw) {}
    ~Catalogue() { ade_catalogue_free(m_raw); }

    Catalogue(const Catalogue &) = delete;
    Catalogue &operator=(const Catalogue &) = delete;
    Catalogue(Catalogue &&other) noexcept : m_raw(std::exchange(other.m_raw, nullptr)) {}
    Catalogue &operator=(Catalogue &&other) noexcept {
        if (this != &other) {
            ade_catalogue_free(m_raw);
            m_raw = std::exchange(other.m_raw, nullptr);
        }
        return *this;
    }

    // Where a dataset lives, or empty when none is configured — which is the
    // ordinary case and not a failure.
    static QString configuredLocation() {
        char *dir = ade_datfiles_location();
        if (!dir) return {};
        const QString path = QString::fromUtf8(dir);
        ade_string_free(dir);
        return path;
    }

    static Catalogue load(const QString &dir) {
        return Catalogue{ade_catalogue_open(dir.toUtf8().constData())};
    }

    explicit operator bool() const { return m_raw != nullptr; }
    size_t count() const { return ade_catalogue_count(m_raw); }
    const AdeCatalogue *raw() const { return m_raw; }

private:
    AdeCatalogue *m_raw = nullptr;
};

// An open disk image.
class Image {
public:
    Image() = default;
    ~Image() { ade_image_free(m_raw); }

    Image(const Image &) = delete;
    Image &operator=(const Image &) = delete;
    Image(Image &&other) noexcept
        : m_raw(std::exchange(other.m_raw, nullptr)), m_error(other.m_error) {}
    Image &operator=(Image &&other) noexcept {
        if (this != &other) {
            ade_image_free(m_raw);
            m_raw = std::exchange(other.m_raw, nullptr);
            m_error = other.m_error;
        }
        return *this;
    }

    // Returns a closed Image on failure; ask `error()` why.
    // `catalogue` may be null: the image is then simply unnamed, at no cost.
    static Image open(const QString &path, const Catalogue *catalogue = nullptr) {
        Image image;
        const QByteArray utf8 = path.toUtf8();
        image.m_raw = ade_image_open(utf8.constData(),
                                     catalogue ? catalogue->raw() : nullptr, &image.m_error);
        return image;
    }

    explicit operator bool() const { return m_raw != nullptr; }
    AdeResult error() const { return m_error; }

    /// Whether the container is a kind of disk image at all.
    bool recognised() const { return ade_image_recognised(m_raw); }

    QString container() const {
        const char *s = ade_image_container(m_raw);
        return s ? QString::fromLatin1(s) : QString{};
    }
    QString volumeAbsent() const {
        const char *s = ade_image_volume_absent(m_raw);
        return s ? QString::fromLatin1(s) : QString{};
    }
    quint64 size() const { return ade_image_size(m_raw); }
    bool hasVolume() const { return ade_image_has_volume(m_raw); }
    QString volumeName() const { return latin1(ade_image_volume_name(m_raw)); }
    quint32 rootBlock() const { return ade_image_root_block(m_raw); }
    size_t findingCount() const { return ade_image_finding_count(m_raw); }
    // What the dataset called it, decided when it was opened. Empty when no
    // dataset was loaded, or when this disk is not in it.
    QString identified() const { return latin1(ade_image_identified(m_raw)); }

    // The device's partitions, or a closed handle for an image that has none —
    // which is most images, and not a fault.
    Partitions partitions() const { return Partitions{ade_partitions_open(m_raw)}; }

    // Reading takes a partition index, or ADE_WHOLE_IMAGE for an image holding
    // its own volume. A partition is not merely an offset: it carries its own
    // block size and reserved count, and the rootblock is computed from both,
    // so the engine resolves it rather than the GUI adding numbers together.
    Listing list(quint32 partition, quint32 block) const {
        return Listing{ade_dir_open(m_raw, partition, block)};
    }
    // Every entry on the volume, flattened. The engine does the traversal
    // because doing it here would mean reimplementing cycle detection.
    Listing walk(quint32 partition) const { return Listing{ade_walk_open(m_raw, partition)}; }
    Buffer read(quint32 partition, quint32 block) const {
        return Buffer{ade_file_read(m_raw, partition, block)};
    }

    /// Write every file into `dir`. Returns false if nothing could be read.
    ///
    /// `uint64_t` rather than `quint64` across the call: they are the same
    /// width but not the same type here, and Qt's is `long long` where the
    /// ABI's is `long`.
    bool unpack(quint32 partition, const QString &dir, uint64_t *written,
                uint64_t *skipped) const {
        return ade_unpack(m_raw, partition, dir.toLocal8Bit().constData(), written, skipped) ==
               ADE_OK;
    }

    /// Search the image's bytes for text or hex.
    Search find(const char *pattern, bool text = false, bool ignoreCase = false) const {
        return Search{ade_find_open(m_raw, pattern, text, ignoreCase)};
    }

    /// Raw bytes of the mounted image, for a hex view of the disk itself.
    Buffer readRange(quint64 offset, quint64 length) const {
        return Buffer{ade_image_read(m_raw, offset, length)};
    }

    /// What occupies each block, as runs. Empty when the map is unavailable —
    /// which is not a failure to report: an image whose regions are unknown
    /// still shows its bytes, just without the colour.
    QVector<HexRegion> regions() const {
        QVector<HexRegion> out;
        AdeLayout *layout = ade_layout_open(m_raw, ADE_WHOLE_IMAGE);
        if (layout == nullptr) return out;
        const size_t count = ade_layout_count(layout);
        out.reserve(static_cast<int>(count));
        for (size_t i = 0; i < count; ++i) {
            AdeSpan span;
            if (ade_layout_span(layout, i, &span) != ADE_OK) continue;
            out.append(HexRegion{span.offset, span.offset + span.length,
                                 static_cast<int>(span.region), span.owner_block,
                                 latin1(span.owner)});
        }
        ade_layout_free(layout);
        return out;
    }

private:
    AdeImage *m_raw = nullptr;
    AdeResult m_error = ADE_OK;
};

}  // namespace ade
