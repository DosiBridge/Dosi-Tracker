import Foundation

extension String {
    /// The longest prefix whose UTF-16 length is at most `limit`, never
    /// splitting a character. The server's `[MaxLength]` counts UTF-16 code
    /// units (.NET string length), so client-side limits do too.
    public func truncatedUTF16(_ limit: Int) -> String {
        guard utf16.count > limit else { return self }
        var result = ""
        var used = 0
        for character in self {
            let size = character.utf16.count
            if used + size > limit { break }
            result.append(character)
            used += size
        }
        return result
    }

    /// Trimmed, or nil when nothing is left.
    var nonEmptyTrimmed: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

/// Percent-encoding for query values and form bodies, byte for byte the same
/// as the Windows agent's `urlencode`: only `A-Z a-z 0-9 - _ . ~` stay as-is,
/// everything else (including `+`, which ASP.NET would read as a space, and
/// spaces) becomes `%XX` of its UTF-8 bytes.
public enum PercentEncoding {
    public static func encode(_ value: String) -> String {
        var out = ""
        out.reserveCapacity(value.utf8.count)
        for byte in value.utf8 {
            switch byte {
            case 65...90, 97...122, 48...57, 45, 95, 46, 126: // A-Z a-z 0-9 - _ . ~
                out.append(Character(Unicode.Scalar(byte)))
            default:
                out.append("%")
                out.append(hexDigit(byte >> 4))
                out.append(hexDigit(byte & 0x0F))
            }
        }
        return out
    }

    /// `application/x-www-form-urlencoded` body of `fields`, in order.
    public static func form(_ fields: [(String, String)]) -> Data {
        let pairs = fields.map { encode($0.0) + "=" + encode($0.1) }
        return Data(pairs.joined(separator: "&").utf8)
    }

    /// A query string (without the `?`) of `items`, in order.
    public static func query(_ items: [(String, String)]) -> String {
        items.map { encode($0.0) + "=" + encode($0.1) }.joined(separator: "&")
    }

    private static func hexDigit(_ nibble: UInt8) -> Character {
        let digits: [Character] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F"]
        return digits[Int(nibble)]
    }
}
