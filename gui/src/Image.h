// RAII wrappers over the C ABI (D-001).
//
// The ABI hands out raw pointers with explicit free functions, which is right
// for a C interface and wrong to use directly from C++. Everything below owns
// exactly one handle and releases it once, so no path through the UI can leak
// or double-free — including the ones that throw.

#pragma once

#include <ade.h>

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
    static Image open(const QString &path) {
        Image image;
        const QByteArray utf8 = path.toUtf8();
        image.m_raw = ade_image_open(utf8.constData(), &image.m_error);
        return image;
    }

    explicit operator bool() const { return m_raw != nullptr; }
    AdeResult error() const { return m_error; }

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

    Listing list(quint32 block) const { return Listing{ade_dir_open(m_raw, block)}; }
    // Every entry on the volume, flattened. The engine does the traversal
    // because doing it here would mean reimplementing cycle detection.
    Listing walk() const { return Listing{ade_walk_open(m_raw)}; }
    Buffer read(quint32 block) const { return Buffer{ade_file_read(m_raw, block)}; }

private:
    AdeImage *m_raw = nullptr;
    AdeResult m_error = ADE_OK;
};

}  // namespace ade
